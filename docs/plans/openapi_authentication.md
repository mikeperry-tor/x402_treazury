# OpenAPI authentication

## Intent, baseline and scope

Implement generic host-managed authentication only where an operation's effective
OpenAPI security requirements declare it. The separate
[manual raw-header contract](../agent-sources.md#manual-raw-headers) permits explicit caller headers
without automatic credential lookup; it is not a prerequisite for this mechanism.
Clear reusable authentication on wallet
rotation by default. Allow a provider/source option to preserve authentication
across rotation, explicitly accepting provider-visible linkage between payers.

Starting implementation baseline: `881a95e49a714898f3ee8856072feb8d5846858b`.
Reconcile intervening changes before implementation; preserve unrelated working-tree
files. This is an implementation plan, not implemented behavior or authorization
for live purchases, credential acquisition, mailbox creation or sending email.

Current behavior:

- `src/catalog/headers.rs` ignores OpenAPI `Authorization` header parameters and
  rejects authentication/payment headers supplied as routed business headers.
  Catalog generation does not compile OpenAPI security requirements into a
  runtime authentication policy.
- `src/discovery/raw.rs` accepts method, URL, query, JSON body and explicit manual
  headers, including Authorization, without registration. Validated manual headers
  are separate from routed business headers, caller-private, excluded from paid
  relays and pinned to the initial payer. They provide no host credential lookup,
  storage, automatic injection, agent-held-secret clearing or response redaction.
- `src/payment.rs` owns identity-bound challenge/sign/retry, response collection
  and accepted-work completion. `src/output.rs` normally returns JSON text as
  received; private response credentials currently have no capture/redaction path.
- Managed selection can choose ACTIVE, RETAINED or READY wallets. Advisory
  selection is not a published rotation. `consume_reserve` in
  `src/rotation/store/capacity.rs` publishes READY -> ACTIVE and the previous
  ACTIVE -> RETAINED inside the authorization-journal transaction.

The first implementation covers HTTP bearer, HTTP Basic and header API keys, with
host-provided credentials and explicitly configured response capture. OAuth/OIDC
acquisition, SIWX signing, query/cookie API keys, mutual TLS and other HTTP schemes
remain explicitly unsupported until their complete contracts are implemented.
These are implementation boundaries, not claims that those OpenAPI schemes are
invalid. Generic auth does not itself implement browser/CDP clients, asynchronous
job orchestration or provider account transfer.

Owning guides to read before implementation: [configuration](../configuration.md),
[source authority](../agent-sources.md#configuration-and-authority),
[architecture](../architecture.md), [wallet rules](../wallet-rotation.md#best-effort-wallet-rules),
[wallet observation](../wallet-rotation.md#wallet-observation-policy),
[network policy](../network-egress.md), [resource policy](../local-resource-limits.md),
[provider policy](../../providers/README.md#provider-policy),
[development](../development.md), [testing](../testing.md) and
[complexity review](../../tests/COMPLEXITY.md#review-discipline).

## OpenAPI is the authentication boundary

Use the repository's supported OpenAPI versions and resolve local security-scheme
references without introducing another document fetcher. Preserve the standard
[Security Requirement](https://spec.openapis.org/oas/v3.1.1.html#security-requirement-object)
and [Security Scheme](https://spec.openapis.org/oas/v3.1.1.html#security-scheme-object)
semantics:

- Operation security overrides root security. Absent operation security inherits;
  `security: []` removes it. An empty requirement object permits anonymous access.
- Requirement entries are alternatives; schemes within one entry are required
  together. Select one complete supported alternative deterministically. Do not
  send credentials for every alternative or partially satisfy an AND requirement.
- Prefer an available anonymous alternative without attaching credentials. Missing
  credentials and unsupported alternatives are distinct errors when no complete
  alternative is available. Preserve required scopes; do not silently discard them.
- Reject conflicting wire slots, unresolved declarations and scheme bindings that
  do not exist. Unsupported authentication disables only affected operations, not
  independent public tools or valid alternatives.
- Where an effective security scheme owns a header also declared as an ordinary
  `in: header` parameter, suppress that business argument and reject caller values
  for the slot, case-insensitively. Selecting an anonymous alternative does not
  restore an agent-supplied auth argument. Keep ordinary nonauth custom headers
  for operations whose effective security does not assign them to authentication;
  explicitly test root overrides with `security: []`.
- An ordinary header parameter, guide prose, host name, response field name or
  `WWW-Authenticate` challenge cannot grant permission to attach credentials.
  Keep the OpenAPI rule ignoring `Authorization` parameter declarations; use
  `securitySchemes` and effective `security` instead.

Existing x402 headers and payment negotiation remain runtime-owned. An OpenAPI
scheme describing x402 is not an ordinary secret binding, and generic auth cannot
replace its signer, amount authority or retry policy. Define an explicit protocol
classification at the first design gate; reject ambiguous collisions rather than
mapping arbitrary vendor scheme names to x402 or treating every API key as payment.
Absent security declarations leave current x402 behavior intact without enabling
additional authentication. API-key-free gateways whose schemas contain inherited
Authorization parameters must not acquire a new bearer requirement from them.

## Configuration and credential sources

Proposed provider/source setting, inherited with ordinary scalar replacement:

```toml
[auth]
clear_on_wallet_rotation = true
```

Omission means true. Setting false preserves existing credential activation across
wallet changes only for that source binding. A deployment may override the provider
value. Expose the effective value and origin in `config show`, with a concise
explanation that false can link payers through the same account/resource. No secret
values appear in config inspection, routing previews or tool schemas.

Credential bindings reference exact OpenAPI scheme IDs. Use host-controlled secret
references for pre-existing credentials; never store literal credentials in bundled
TOML, source registries or agent arguments. Define the credential shape for bearer,
Basic username/password and API-key values without guessing encodings or formats.

OpenAPI does not standardize obtaining a resource capability from arbitrary JSON.
Provider/source response-capture configuration may identify an exact operation,
successful response branch, declared response field, resource identifier and target
scheme. This supplies the missing acquisition relationship; it cannot invent a new
scheme, destination or authenticated operation. Require explicit capture bindings,
not heuristics for fields called token/secret and not execution of vendor prose.
Missing or incompatible acquisition declarations keep the integration unsupported.
Ordinary remote schema refreshes cannot silently add capture authority or broaden
the allowed use of a previously captured credential.

Return an opaque auth/resource reference instead of the captured secret. Bind it to
the authenticated listener principal, source identity, declared scheme, resource,
approved HTTPS origin/operations and acquiring payer provenance. Possession of a
reference does not bypass principal checks. Listeners sharing a wallet or catalog
do not thereby share credential access; agents sharing one listener already share
that listener's authority. Provide an equivalent explicit local principal for stdio.
Resolve multiple credentials through a reference or unambiguous resource binding;
never pick an arbitrary token for a scheme.

## Rotation and concurrency contract

Clearing means invalidating reusable authentication for future calls, not claiming
remote revocation or guaranteed physical erasure. Keep only the private information
needed to complete/recover accepted work. An old reference reports that its auth
was cleared; do not hide this as an empty mailbox or automatically buy a replacement.

Default behavior must cover both forms of identity change:

1. A committed managed ACTIVE replacement invalidates all pre-rotation reusable
   auth in affected source/listener bindings whose effective option is true,
   including credentials associated with previously retained payers. Static
   signer replacement is the corresponding event
   for static bindings. Reconcile persisted identity/invalidation on restart before
   any authenticated I/O; missed process-local notifications cannot revive auth.
2. A per-call price handoff or insufficient-funds traversal may choose another
   wallet without publishing ACTIVE replacement. Before that wallet's first
   unsigned request, it cannot inherit the prior wallet's auth under the default.
   Re-evaluate the declared alternatives using that payer's eligible credentials
   (including an anonymous alternative), or report auth unavailable before I/O.
   Advisory candidate inspection does not itself clear unrelated bindings.

Tor scope/client-cache replacement, balance refresh, catalog refresh and allocation
of an unused READY wallet are not wallet rotation. Same-wallet unsigned retries
retain their exact credential snapshot and existing retry authority.

Accepted operations may finish with their captured credential and original payer
through concurrent rotation. That snapshot cannot transfer to a different-payer
attempt. An acquisition whose own admission makes its payer ACTIVE must be bound
to the resulting committed identity, not accidentally invalidated by its own
promotion. A response arriving after a later rotation can be privately retained
for accepted-work recovery but cannot republish reusable old-wallet auth. Returning
to a retained wallet, reloading environment values, restarting or re-registering a
source must not resurrect a cleared activation. Explicit fresh authentication is
required; it grants no automatic paid retry or purchase authority.

With `clear_on_wallet_rotation = false`, preserve auth for future eligible payers
and use each attempt's normal identity-bound transport. This does not establish
vendor support for another payer, override wallet-owned account restrictions,
grant spending authority or hide the resulting linkage. Capture provenance remains
the original payer. A failed auth response does not authorize switching identities,
refreshing tokens, replaying signed work or acquiring a replacement resource.

At the first design gate choose the smallest durable representation that proves
these races, including concurrent active promotion, static replacement, late
capture and restart. Reuse authoritative wallet transitions where possible; do not
create a second wallet lifecycle or monetary ledger. If an auth invalidation
generation is necessary, justify its exact stale-publication invariant and keep it
local to credentials. Do not add counters or a generic workflow framework merely
to notify auth of wallet changes.

## Request, response and storage integration

Keep authentication distinct from business headers and payment authority. Resolve
the authorized operation and payer before attaching secrets. Inject through the
existing `src/network.rs` transports and `PaidClient` request path; no new vendor
HTTP clients, direct Tor fallback or authentication preflight on every call.

Generated tools, on-demand invocation and host-authenticated raw calls use the same
resolver. Extend raw calls with source/operation and opaque auth references only
as necessary. Host-authenticated raw URLs/methods resolve uniquely to an accessible
registered/configured OpenAPI operation, including its effective server URL, path
parameters and security requirements. Reject ambiguous/unmatched requests before
credential lookup or network I/O. Existing unauthenticated raw calls remain usable
without registration. Manual raw Authorization belongs to the separate manual-header
contract and cannot be combined with host-managed auth in one call. Reject mixed
modes before credential lookup or I/O. Do not add a process-wide automatic bearer
cookie jar; raw authority alone grants no host credential access.

Do not forward secrets to discovery/help/spec URLs, other resources, redirects or
paid relays. Preserve current redirect policy and relay credential exclusions;
mark auth separately so a pre-injection relay check cannot miss it. Declared API-key
headers must use the same exclusion. Reject caller-auth conflicts and protected
slot collisions before dispatch. Every request still requires ordinary public
destination checks and applicable endpoint/source authority.

Capture and redact declared response credentials inside retained accepted-work
ownership, before output conversion, cache publication, diagnostics or result
delivery. Secret bytes must not enter ordinary tool text, JSON metadata, errors,
debug formatting, routing previews, registry snapshots, reports or payment evidence.
Malformed secret-bearing responses must also fail without exposing their bodies.
Review low-level public `PaidClient` response consumers so no alternative result
path bypasses the same enabled capture policy.

Choose a minimal encrypted, owner-controlled credential store usable with both
static-only and treasury deployments. Reuse suitable existing cryptographic and
private-filesystem primitives without assuming every deployment has a treasury
key. Define explicit key references, versioning, atomic publication/invalidation,
restart behavior and Linux/macOS/Windows guarantees at the first design gate.
Inspection must not unlock/open credential storage. No secret values in source
configuration/registry persistence. Store failures affect the operation concerned;
they cannot return a usable-looking handle or trigger another purchase.

Waiter cancellation does not cancel received-secret persistence. Shutdown drains
accepted capture work. Loss of the response before capture still needs a documented
vendor recovery contract; neither payment journals nor idempotency keys establish
recoverability. This feature adds no automatic paid replay, resource renewal,
cleanup request, polling, token refresh, arbitrary lifetime, capacity limit or
whole-work deadline. Honor documented expiry when supplied; do not invent a TTL.
Source refresh/removal must neither erase accepted-work recovery nor expand old
grants; removed/revoked source authority denies new use, and name reuse grants no
old-secret access.

## Ordered implementation commits and review gates

Each identifier is a coherent commit boundary. Keep sources frozen during checks.
Before A01, obtain independent read-only design review of the whole proposal and
the concrete storage/rotation representation. Resolve material findings before
building on them. The coordinating author integrates findings and owns completion.

### A01 — Compile OpenAPI authentication requirements

Add normalized effective-security metadata to catalog operations and configuration
bindings. Implement inheritance, alternatives/conjunctions, anonymous overrides,
local references, unsupported-case diagnostics and payment-scheme separation.
Do not expose secret capture or enable host credential transmission in this commit.
Reserve effective authentication header slots from business arguments and document
the migration where a previously ordinary header is declared as authentication.

Acceptance: synthetic parsing/config tests prove every semantic branch, immutable
metadata propagation through static and dynamic catalogs, source override origins,
paired parameter/security declarations (including case variants, anonymous
alternatives and `security: []`), secret-free inspection and unchanged x402 behavior.
Add the implemented
metadata contract to `docs/authentication.md` and `docs/configuration.md`.

### A02 — Deliver host-managed auth with rotation and response capture

Implement credential provision/capture, private persistence, opaque references,
operation-bound injection, redaction, cancellation ownership and default rotation
invalidation together. Include the false provider option and complete same-payer
retry/different-payer handoff behavior. Do not land an enabled purchase/capture path
before its credential use, clearing and recovery boundary exist.

Relevant consumers: `src/payment.rs`, its submission/ownership paths, catalog-bound
tools, `src/output.rs`, listener ownership, static signer replacement, managed
admission and `src/rotation/store/capacity.rs`. Keep requests in `src/network.rs`.

Acceptance: focused synthetic tests cover supported schemes and wire collisions;
two listeners sharing a payer; two resources/sources sharing a scheme; absent,
expired and invalidated credentials; default and false-option rotation; initial
promotion, concurrent free reads, handoff before unsigned I/O, late responses,
restart and environment reload; cancellation during capture; disk/commit failure;
secret-free outputs/errors/logs; and no unauthorized request, signature, funding,
credential forwarding or duplicate acquisition. Assert actual tests execute.

Update `docs/authentication.md`, configuration, architecture, wallet rotation,
network egress, development diagnostics and testing guidance with this commit.
Add AGENTS.md links to the owning authentication guide and relevant read-first rows.

Gate G1, after A02: independent architecture review of A01/A02 and all fixups,
including complete catalog -> owner -> payer -> transport -> response -> storage ->
output chains. Verify rotation/restart ordering, principal separation, relay policy
and no hidden credential resurrection. Compare CogC/CycC with the agreed baseline;
review unchanged payment/admission hotspots and concrete simpler representations.
Exit only after material findings and their regressions are resolved.

### A03 — Expose constrained authenticated raw calls

Reuse the shared resolver for an explicit source/operation/auth reference on raw
calls, preserving existing anonymous raw behavior. Carry the same owner, source
wallet policy, operation security and response capture through raw execution.

Acceptance: shared catalog/raw tests prove exact operation resolution, parameterized
paths and effective servers; deny other origins/resources, ambiguous operations,
wrong principals, forged references and arbitrary auth headers before I/O. Test
both direct and on-demand consumers, relay exclusion and secret-free previews.
Update `docs/agent-sources.md`, configuration and `docs/authentication.md`.

### A04 — Integrate, validate and remove this completed plan

Review provider compatibility policy against the implemented mechanism. Do not
enable Ausca, agmail or owned-mailbox purchases merely because auth now exists:
their schema, lifecycle and lost-response contracts still need independent review.
Use small synthetic contracts for generic behavior; add provider fixtures only for
a named real quirk or authored transformation. No paid qualification is implied.

Gate G2, before cleanup: independent final integration review of A01–A03 and every
fixup, their consumers, docs and remaining limitations. Review repeated fixup
causes for wider hazards, missing tests or opportunities to delete responsibilities.
Earlier per-commit approval does not exempt code from combined review. Reopen G1
concerns if later changes affect them. Exit requires resolved material findings,
source-matched validation and complexity evidence, and verified permanent guidance.

Verify `docs/authentication.md` owns the enduring rules; configuration owns exact
settings/defaults; wallet/network/source guides own their respective boundaries;
providers/README.md and CAVEATS.md describe actual provider support; testing owns
qualification claims. Verify AGENTS.md and `docs/README.md` link the owning guide.
Remove this plan and fix its index links in A04 only after that handoff. Replace
incoming sibling-plan links with permanent owning-guide links regardless of which
plan finishes first. Independently
review the exact cleanup SHA and the final combined branch as well; retain no
completed-plan archive and erase no private financial or credential evidence.

## Validation and exact-SHA discipline

Before substantive Rust edits, capture CogC/CycC against the starting baseline
using the pinned analyzer. At G1/G2 compare changed functions and complete call
chains, inspect new/worsening warnings and unchanged high-risk hotspots, and record
explicit simplification/disposition decisions under the complexity guide. Generated
reports stay under ignored `target/`; helper extraction alone is not simplification.

Run focused tests through Makefile entrypoints first, selecting socket-capable
execution before localhost fixtures. Likely owners include `tests/headers.rs`,
`tests/config.rs`, `tests/managed_config.rs`, `tests/network_audit.rs`, payment and
rotation module tests, and discovery raw-call tests. Select exact test names after
implementation and verify nonzero counts, forbidden side effects and test wiring.
Use public unfunded fixture keys and temporary secret stores, never real secrets.
Run applicable format/Clippy checks and the full `make check` at final integration;
repeat broader checks only for subsequent changes or unresolved findings. Run any
requested feature configurations sequentially. Add no arbitrary test-derived
runtime limits; if implementation needs a justified resource-policy change, update
both resource and timer inventories in the introducing commit.

Independently review every exact implementation SHA and relevant consumers; the
reviewer must not be its author. Give reviewers user intent, this baseline, scope,
preserved invariants, exact test counts and complexity evidence. Require them to
verify claims and challenge unnecessary enforcement. Address findings using
`git commit --fixup=<introducing-SHA>`; independently review every fixup and retain
unsquashed history through integration. Material follow-up changes reopen affected
gates and receive revalidation. If independent review tooling is unavailable,
disclose it and do not claim independent approval.
