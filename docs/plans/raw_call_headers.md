# Manual headers for raw calls

## Intent and baseline

Allow agents to supply documented headers to `x402_treazury_raw_call`, including
custom `X-*`, a caller-owned `Idempotency-Key` and explicit `Authorization`.
Keep this manual escape hatch separate from automatic host-managed authentication.
It must work without registering an OpenAPI source and must not require the
[OpenAPI authentication plan](openapi_authentication.md) to be implemented first.

Starting baseline: `9460c9f6a5995dc090770157d995ae976f020a08`. Reconcile intervening
changes before implementation. Preserve unrelated files and local changes. This
plan grants no live execution, spending, credential acquisition or email authority.

Current call chain: `src/discovery/raw.rs` strictly accepts method, URL, query and
JSON body; it constructs an empty `RoutedRequest.headers`. `src/payment.rs` builds
one request and preserves it through challenge/sign/retry and eligible payer
handoffs. `src/catalog/headers.rs::business_name` rejects Authorization and
transport/payment headers, while ordinary routed business headers already work.
Catalog `Idempotency-Key` parameters are automatically generated per invocation;
raw calls currently generate no key. Relay adapters recognize a finite list of
credential header names; they cannot identify arbitrary secret-bearing `X-*` names.

Read the owning [raw-call contract](../agent-sources.md#direct-calls-to-x402-urls),
[header contract](../configuration.md#openapi-header-parameters-and-idempotency),
[architecture](../architecture.md), [wallet rules](../wallet-rotation.md#best-effort-wallet-rules),
[wallet observation](../wallet-rotation.md#wallet-observation-policy),
[network policy](../network-egress.md), [resource policy](../local-resource-limits.md),
[development/logging](../development.md), [testing](../testing.md) and
[complexity discipline](../../tests/COMPLEXITY.md#review-discipline).

## Proposed interface and boundaries

Add an optional `headers` object with string values. Omitted or empty means the
current raw-call behavior. For example, using fictional values only:

```json
{
  "method": "POST",
  "url": "https://api.example.com/action",
  "headers": {
    "Authorization": "Bearer example-token",
    "Idempotency-Key": "example-operation-identity",
    "X-Workspace-Id": "example-workspace"
  },
  "body": { "action": "example" }
}
```

The header map is explicit invocation input, not trusted input. Validate names and
values using HTTP types, reject CR/LF injection and case-insensitive duplicate
names, and reject nonstring values. Reject exact duplicate JSON keys if they can
reach this layer without already being collapsed by the MCP JSON parser; document
the actual parser boundary rather than promising detection of information already
lost. Do not serialize nested objects/arrays or silently join repeated fields.
Do not add arbitrary header-count, byte, timeout or concurrency limits.

Allow ordinary business headers, custom `X-*`, manual `Idempotency-Key` and, in H02,
Authorization. Keep runtime-owned x402 headers, Host, proxy/connection/framing
headers, Cookie/Cookie2, Accept and Content-Type reserved under the existing header
boundary. Preserve JSON body encoding and existing output negotiation; arbitrary
body/content negotiation and cookie sessions are outside this change. Validate
all headers before network I/O, signing or relay spending. Error messages may name
the offending header but must not include its value. Mark every manually supplied
HTTP HeaderValue sensitive for Debug/diagnostic formatting, including ordinary
custom headers, while checking that earlier string-map formatting cannot leak it.

Authorization is a raw-call-only exception carried through an explicit internal
request policy, not a global relaxation of `business_name` or a forged business
header. Keep generated catalog tools and direct Rust `RoutedRequest` construction
under their existing restrictions. Carry only the minimum provenance needed to
distinguish a validated manual raw request from ordinary routing. Do not add
another HTTP client or bypass `src/network.rs`.

## Idempotency and accepted work

Send the supplied `Idempotency-Key` value unchanged. Do not generate or replace it,
apply the catalog UUID schema to it, or remember it across calls. Reusing one key
in a later call is an explicit caller decision, not an automatically resumed call.
Existing generated OpenAPI tools keep their automatic per-invocation keys.

Preserve all supplied header values through the already-authorized same-payer
unsigned retries and signed dispatch. A key grants no retry authority, guarantees
neither vendor deduplication nor a single x402 debit, and does not permit automatic
replay after a signed timeout, failed output conversion or uncertain outcome.
Two invocations with the same key may still create separately chargeable payment
authorizations. Provider recovery must follow its actual documented contract.

Retain current cancellation/shutdown ownership: an accepted call completes its
request/response processing even if its waiter disconnects. Add no host credential
store, cross-call deduplication, token acquisition/refresh or recovery journal.
Do not treat an authenticated state-changing request as payment-gated merely
because it uses raw_call; preserve existing method/retry restrictions and test
that introducing headers creates no new unsigned or signed replay authority.

## Confidentiality, relays and wallet identity

Treat every nonempty manual header set as caller-private for forwarding and payer
continuity. Arbitrary names can hold credentials; a name denylist or the `X-`
prefix cannot establish sensitivity. This intentionally applies even when a call
only supplies an idempotency key or harmless business header. The concrete tradeoff
is fewer fallback opportunities in exchange for avoiding hidden disclosure; it
adds no resource quota, funding gate or timer.

- Do not send a request carrying manual headers through any paid relay. Enforce
  this before adapter preparation/payment, not only after matching known credential
  names. Preserve existing no-redirect behavior and public HTTPS/origin checks.
  Header-free raw calls retain existing configured relay behavior.
- Select the initial existing payer using normal admission, then keep the call on
  that identity. Before any price-based handoff or recognized insufficient-funds
  traversal would issue I/O under another payer, stop with a safe operation-scoped
  error explaining that private headers cannot follow another wallet. Preserve
  any earlier possible-charge evidence. Do not sign or probe the next wallet,
  allocate a wallet, wait for funding or alter the pool's ordinary selection rules.
- Same-payer eligible Tor retries may use the existing fresh isolation scopes;
  a new Tor scope is not a new payer. Concurrent ACTIVE replacement or static
  signer replacement must not abandon an accepted call using its captured payer.
  Header-free calls retain their current best-effort wallet traversal.

The initial implementation adds no cross-payer override. If later needed, design
an explicit continuity permission with its linkage consequences independently;
the automatic-auth provider option must not silently authorize manual raw headers.

Manual secrets necessarily appear in model-authored arguments and may be retained
by the client in conversation/tool history. They cannot satisfy a provider's
host-only capability contract. No new host persistence is added, but accepted work
retains the exact request until completion. Do not log, Debug-print, cache, persist
in source registries, or expose manual header values in server diagnostics/errors
or route previews. Test the existing request debug/error paths and any HTTP cache
consumer rather than assuming request headers are absent there.

This feature cannot clear secrets held by an agent. The agent may send a token
again after wallet rotation, linking the old and new identities to the provider.
Do not describe manual headers as enforcing the automatic-auth plan's default
clearing guarantee. Generic raw response bodies retain their existing contract;
this feature does not infer or redact credential fields in arbitrary provider
JSON. An upstream response can echo a supplied credential into ordinary raw output;
manual mode makes no response-secret redaction guarantee. State that distinction
in agent-facing guidance.

## Relationship to OpenAPI-managed authentication

OpenAPI-only authentication remains the rule for automatic credential capture,
lookup and injection. Manual raw headers are an explicit, separately authorized
exception for caller-provided requests, with no automatic credential access and
no claim that a guide or URL supplies host-managed authentication authority.

When both mechanisms exist, do not merge manual headers and host-managed auth
references in one call. Reject conflicting modes before credential lookup or I/O.
Managed raw calls resolve their registered OpenAPI operation; manual raw calls
continue to use documented URLs without registration. Reuse request construction,
transport and payment ownership wherever possible, while retaining the distinct
authority. Neither mode broadens listener/source ownership or spending limits.
Make these boundaries explicit in both plans and then in permanent guides; neither
plan may retain contradictory claims about raw Authorization support.

## Ordered commits and independent review

Before H01, obtain independent read-only design review of this scope and the exact
internal policy representation. Challenge the blanket private-header treatment
against a more complex sensitivity API, and choose the simplest representation
that protects arbitrary custom credentials. Resolve material findings before
building on affected decisions. Capture the Rust complexity baseline before edits.

### H01 — Ordinary headers and manual idempotency with complete safeguards

Add the optional raw header map and strict validation for ordinary custom headers
and Idempotency-Key. Deliver caller-private request provenance, relay exclusion
and initial-payer continuity in the same commit; arbitrary X-* credentials must
not become transmittable before these protections exist. Authorization remains
rejected until H02. Do not change generated-tool idempotency or header-free calls.

Relevant consumers: `src/discovery/raw.rs`, discovery tool schema/descriptions,
`src/catalog/headers.rs`, `src/payment.rs`, its payer handoff/submission consumers,
`src/discovery_relay/api.rs` and adapters, public `PaidClient` paths and network
factory. Avoid competing source edits or builds during validation.

Acceptance: focused synthetic tests prove exact header/idempotency delivery,
omitted/empty-header compatibility, no-body versus null-body behavior, validation
before I/O, case-insensitive duplicate rejection, reserved names, invalid values,
custom secret-looking and innocuous-looking names, and value-free diagnostics.
Exercise normal and on-demand MCP consumers and endpoint authority. Assert no
relay request/signature/charge for nonempty manual headers; no second-payer I/O
on both price handoff and structured insufficient funds; no wallet allocation or
funding; same-payer permitted retries preserve header bytes; header-free traversal
and relay fallback still work. Verify tests are wired and execute nonzero counts.

Update `docs/agent-sources.md`, `docs/configuration.md`, `docs/network-egress.md`,
`docs/wallet-rotation.md` and affected test guidance with the implemented boundary.
Document the inability to infer whether an arbitrary custom header is a secret.

### H02 — Explicit manual Authorization

Permit Authorization only through the validated manual-raw path. Reject protected
payment/transport slots regardless of case. Keep direct routed metadata and
generated tools unable to inject Authorization through business headers. Preserve
H01's private-header and same-payer rules for all manual header names.

Acceptance: tests prove Authorization delivery with no OpenAPI registration, no
credential storage/automatic injection, global business-header rejection, no
credential forwarding on unsigned failure, no accidental payer handoff and no
value-bearing logs/errors. Cover an unsigned authenticated success, signed
challenge/retry, terminal/uncertain outcomes, cancellation and concurrent wallet
rotation while progress continues. These use synthetic contracts, not paid vendor
mailboxes. If managed auth has landed, test mixed-mode rejection and principal
isolation through its real consumers.

Update source/configuration/network guides and the OpenAPI-authentication guide if
present. Add or update the relevant AGENTS.md read-first links to the permanent
raw-header/manual-auth contract, without duplicating detailed rules in AGENTS.md.

Gate R1, after H02: independent architecture review of H01/H02 and every fixup
alongside complete listener -> raw validation -> request -> initial payer ->
challenge/handoff -> signed dispatch -> relay/output call chains. Verify positive
behavior and absence of forwarding, second-wallet I/O, secret diagnostics and new
replay authority. Inspect relevant public API entry points and default header-free
consumers. Compare CogC/CycC to the agreed baseline, inspect unchanged payment-loop
hotspots, and evaluate concrete simplifications before retaining new control flow.
Exit only after material findings and exposed regression gaps are resolved.

### H03 — Final integration and documentation handoff

Gate R2, before plan removal: independently review all implementation commits and
fixups, final tests, full consumers and documentation together, including interaction
with any OpenAPI-auth implementation. Revisit recurring fixup causes for a wider
audit, simpler representation or missing coverage; per-commit approvals do not
exempt code from integration review. Later material changes reopen R1 concerns
and receive appropriate revalidation.

Verify permanent destinations: source guide owns raw-call arguments/authority;
configuration owns manual versus generated idempotency and auth modes; network
egress owns forwarding/isolation; wallet rotation owns payer continuity; development
owns safe diagnostics; testing owns fixture/qualification limits. Confirm AGENTS.md
and documentation-index links lead to these current rules. Update the sibling auth
plan only for genuinely unfinished work, keeping no completed-plan archive.

After R2 exits with resolved findings and source-matched evidence, remove this plan
and repair links in H03, replacing incoming sibling-plan links with links to the
permanent owning guides regardless of which plan finishes first. Independently
review the exact cleanup SHA and final
combined branch after removal. Cleanup must not erase private financial evidence,
credentials held elsewhere or accepted-work records, and grants no replay authority.

## Validation and commit discipline

Use Makefile entrypoints and focused tests first. Likely owners are discovery's raw
tests, header tests, payment/handoff/ownership tests, relay tests and network audit.
Select actual test names after implementation, record nonzero test counts and
assert forbidden side effects. Use temporary state/public unfunded keys only.
Choose socket-capable execution before localhost fixtures when required; keep
socket-free checks in the ordinary sandbox. Feature configurations run sequentially
with frozen sources/executables.

Run applicable format/Clippy checks, then final `make check` at integration; repeat
broader tests only for subsequent changes or unresolved concerns. For substantive
Rust changes and R1/R2, run the pinned CogC/CycC analysis against the agreed baseline
and assess complete call chains and relevant unchanged hotspots. Disposition each
warning by evaluating deletion/simplification before justifying necessary retained
responsibilities. Do not extract helpers merely to lower scores. Keep generated
reports under ignored `target/`. No new resource/timer policy is proposed; if a
concrete invariant requires one, update both inventories in its introducing commit.

Independently review each exact implementation SHA and its consumers with a reviewer
other than the author. Supply intent, scope, baseline, preserved invariants, tests
and complexity evidence; ask reviewers to verify claims and challenge unnecessary
enforcement. Address findings with `git commit --fixup=<introducing-SHA>` and
independently review each fixup. Keep history unsquashed through integration.
The coordinator integrates findings and owns completion. If independent tooling
is unavailable, disclose that limitation and do not claim independent review.
