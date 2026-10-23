# Relay disclosure categories and passive pricing

## Intent, baseline and scope

Make relay permission express the information disclosed, and remove speculative
pricing traffic. The operator selected three enable preferences:

```toml
[discovery_relay]
source_discovery = true
api_requests = false
provider_metadata = true
```

The defaults apply when a relay section exists; no section still means no relay.
Remove `serve`, `warm`, `on_demand` and the `search` alias without compatibility
aliases or ignored keys. Keep method lists, adapter formats, declaration-relative
paths, `wallet`/`sources` static binding controls and the existing single-provider
syntax; these are routing/binding settings, not the replaced enable preferences.

Starting implementation and CogC/CycC baseline:
`163a3e790d8887246b00a0b6702bf1a390feff7e`. Generalized fallback is implemented
there. Currently `serve`/`warm` default false and `on_demand` defaults true, with
`search` accepted as an alias. Startup/inspection/warming can proactively probe a
restricted subset of GET operations. A separate passive runtime hint cache learns
validated challenges from actual managed API calls and advises wallet selection.
This plan changes the former behavior while preserving the latter.

Implementation was authorized after the exact plan review at `3250fd98`; P01 and G1 passed through `05ee377b` atop `1f55fc66`; P02 is
in progress. Validation uses local fixtures and public unfunded keys; this plan
does not authorize live paid qualification, real wallet secrets or restarting the
live server. Preserve existing discovery fixes, unrelated tracked/untracked/ignored
files, private evidence and build caches. The current unrelated files include
`TODO`, `discovery_confusion.md`, `examples/deployments/privacy.local2.toml`,
`examples/deployments/privacy.local3.toml` and `run_servers.sh`.

Owning guides: [configuration](../configuration.md#paid-discovery-relay),
[network policy](../network-egress.md), [architecture](../architecture.md),
[agent sources](../agent-sources.md), [resource limits](../local-resource-limits.md),
[best-effort wallet rules](../wallet-rotation.md#best-effort-wallet-rules),
[wallet observations](../wallet-rotation.md#wallet-observation-policy),
[provider policy](../../providers/README.md#provider-policy),
[provider caveats](../../providers/CAVEATS.md), [testing](../testing.md),
[startup tests](../../tests/STARTUP.md),
[qualification reference](../../tests/live/INTEGRATION_REFERENCE.md),
[development](../development.md) and
[complexity discipline](../../tests/COMPLEXITY.md#review-discipline).
AGENTS.md already links these owners; add a link only if a new enduring owner is
actually introduced. Update maintained guidance with each behavior change.

## Proposed contracts

### Categories follow trusted request purpose

| Preference | Included fallback requests | Important boundary |
| --- | --- | --- |
| `source_discovery` | Built-in Coinbase resource search and x402 List browse, best/recommendation and service-detail requests | Covers directory queries/filters/slugs, not requests to the discovered providers. |
| `api_requests` | Ordinary configured/generated API tools and `raw_call`, for representable unsigned GET/HEAD/POST | Includes an application search API, or an ordinary/raw call to a directory URL. Hostname or HTTP method cannot reclassify task data as discovery. |
| `provider_metadata` | Static catalog/schema downloads at startup, inspection where relay use is already authorized, and explicit warming; dynamic imports and refreshes; provider help; explicit usage documents; provider URL/specification discovery probes | No agent API argument map is supplied. Metadata URLs and their query strings can still disclose sensitive information. Proactive endpoint pricing discovery is removed below. |

These switches grant fallback eligibility, not origin-request eligibility or new
payment authority. Other method, representation, destination, response, payer and
financial authority checks still apply. Local bundled-provider searches/listing
remain offline. No separate pricing preference or replacement price probe exists.

Gate each acquisition at its trusted caller boundary. In a mixed `source_details`
workflow, the directory lookup uses `source_discovery`; each subsequent provider
document/probe uses `provider_metadata`. If one step is disabled, do not buy that
step through another category; retain useful independent results or existing
explicit unavailable/error behavior. Do not invent missing provider URLs to work
around a disabled directory lookup. Help remains metadata even when invoked through
a tool wrapper, while ordinary tool invocation remains API work.

Prefer a small closed purpose selector and one shared immutable relay policy and
adapter identity. Do not filter one shared relay handle by a client's API flag and
thereby suppress its document reads; do not create category-specific relay clients,
workers, caches or payer bindings. Do not expose category overrides in agent schemas
or use agent/session fields to select financial owners. Relay-service payment
requests retain recursion prevention; `api_requests=false` must not suppress an
authorized discovery/metadata relay exchange or its ordinary unsigned leg retries.

Keep retained help/import ownership and coalescing semantics. Immutable policy on
the shared relay context should make extra category cache partitions unnecessary;
verify that disabled fallback cannot obtain a newly purchased result through a
different enabled category. Preserve existing identity/policy cache keys and sticky
possible-payment outcomes rather than adding a second permission registry.

`provider_metadata=true` intentionally permits eligible paid catalog fallback in
ordinary serving and explicit non-direct warming without separate `serve`/`warm`
opt-ins. It does not turn inspection into payment, start warming automatically or
enable fallback for `catalog warm --direct`. Preserve each entry point's current
inspection-only and qualification exclusions. Validate listeners/authentication and
bind before paid serving initialization. Retain static source selection and payer
resolution; runtime reads retain their authenticated endpoint/source owner. CLI
refresh stays origin-first, then representability/category checks, then lazy loading
of only the saved existing payer. No new wallet allocation, funding or funding wait.
Configured adapter metadata may still be validated locally when categories are off;
disabled categories must cause zero relay requests or signatures.

### Remove proactive pricing, preserve useful prices

Remove startup/standalone/deferred/inspection/warming endpoint-pricing acquisition,
its scheduler/publication phases, probe response caching, probe-specific relay
contract and dedicated evidence. Do not leave disabled workers, synthetic completed
stages, no-op CLI switches or a second optional pricing workflow.

Remove `--discover-pricing`, `probe_pricing`, `probe_ttl_seconds`,
`probe_concurrency`, `probe_max_endpoints`, `probe_methods` and the `probe_timeout`
field/normalization alias. Old configuration keys and CLI switches fail visibly.
Migrate bundled providers, examples, tooling and authored test configurations in
the same commit. `probe_timeout` currently contributes to general HTTP inactivity:
migrate authored effective values to the supported general setting, preserving
existing general-timeout precedence and values. Keep ordinary `timeout` compatibility
and unrelated environment contracts; do not silently drop or tighten HTTP policy.

Preserve schema/provider `pricing_key`, credit-pricing interpretation/conversion,
authored operation prices and per-payment caps. `pricing::validate` currently also
validates generic HTTP inactivity and credit configuration: move these responsibilities
to their existing configuration/credit owners before deleting the probe validator.
Remove pricing-only description rebuilds while retaining authored descriptions,
filters, recommendations, route/schema identity and complete catalog publication.
Missing authored prices remain unknown; do not label them free or fabricate estimates.

Preserve `src/pricing/runtime.rs` or its equivalent small passive owner, including
operation/revision/pool/transport partitioning, receipt-based lifetime, bounded
cache behavior, out-of-order handling and validation-before-learning. Actual API
calls still perform normal unsigned challenge acquisition, signing, journal-before-
disclosure and permitted insufficient-funds traversal. Runtime hints remain advisory
for existing managed wallet selection; keep their current lifetime, persistence
semantics and no-I/O contract. Do not add persistence or catalog-description
synchronization to replace removed advance prices.

Remove `application_pricing` production and current qualification/live-runner
requirements together: manifest checks, stage collection, inventory preparation,
report models/rendering and provider-readiness decisions must no longer expect a
stage that cannot occur. Preserve actual-call challenge observations, reviewed
amount/case authority, journals and financial correlation. Existing financial
state, pins, events and historical reports remain intact and observation-only;
retain required historical readability at the existing format boundary, without
accepting obsolete live configuration or granting replay. Old probe-cache rows can
remain inert; no destructive state/cache migration or evidence purge is needed.

### Preserved transport and ownership invariants

Keep one origin sequence and ordered method-list traversal, configured Tor retry
count, server Retry-After semantics, network-factory constructors, public/allowed
destinations and remote DNS/no direct fallback. Preserve origin no-follow and the
operator-accepted relay redirect/plaintext trust: absent intermediate-hop evidence
does not exclude an adapter. Never forward target credentials/signatures or sign a
relay-supplied target challenge. Prior target liability still blocks generic relay
fallback; possible relay fees survive conversion, publication and failures.

Keep current qualification authority checks at actual runtime dispatch after waits,
static preparation's separate authority, complete-document rejection and legitimate
API ranges/media, exact captured payers, accepted-work draining and cleanup. Add no
timers, queues, budgets, readiness gates or parallel monetary tracking.

## Ordered implementation commits

1. **P01 — Delete proactive pricing coherently.** Do this first so the new default
   metadata permission never temporarily authorizes the probes being removed.
   Cover `src/pricing.rs` and probe evidence/tests, preserved `pricing/runtime.rs`,
   generic config validation, `src/main.rs` standalone serving and inspection,
   CLI/lib arguments, deployment startup/deferred/static warming and output,
   `Contract::Pricing`/pricing relay slots, and qualification/live-runner producers
   and consumers. Remove only probe-specific cache paths/formats and dead code;
   retain shared catalog caching, challenge parsing and authored price rendering.
   Update provider TOMLs, examples, fixtures, README, configuration/architecture,
   both resource/timer inventories, provider guides and startup/qualification docs
   in this commit. Read the relevant guides before editing each area.

   Acceptance: actual standalone serving, deployment startup/deferred publication,
   inventory inspection and warming make zero unsolicited API endpoint pricing
   requests, with faithful origin/relay counters and usable catalog controls.
   Authored prices/credit conversions and unknown-price output remain correct.
   Real-call 402/signing and hint learn/use still execute; malformed, unselected or
   otherwise invalid offers do not teach hints. Exercise hint identity/receipt TTL,
   concurrent calls, cancellation and accepted-work/store cleanup. Preserve the
   meaning of publication tests using authored metadata rather than deleting their
   ownership assertions. Verify removed fields/flags reject, general timeout and
   credit validation remain, current qualification runs need no pricing stage, and
   historical evidence is readable without mutation or renewed authority.

2. **P02 — Replace enable flags with disclosure categories.** Implement parsing,
   defaults, canonical inspection, trusted-purpose routing and all consumers in
   one coherent change. Cover relay policy/loading, static serving/warming,
   `PaidClient`, document reads/help, source import/refresh, CLI saved-owner refresh,
   Coinbase and built-in x402 List requests, raw/generated calls and mixed
   `source_details` workflows. Update declarations, examples, README and the owning
   configuration/network/agent/architecture guides together. Preserve all method
   lists, binding rules and accepted adapter trust; remove old booleans/aliases.

   Acceptance: default configured policy has discovery+metadata positive relay
   controls and API origin-only controls. Independently flip all three preferences
   and assert physical request purpose/counts and forbidden signatures/fees across
   the listed consumers. Include mixed directory→provider probes, provider help
   through an API wrapper, raw/registered directory URLs, API POST data and no
   category bypass through method/hostname/client reuse. Cover default-on startup,
   explicit warming versus direct warming, offline inspection and qualification
   exclusions, disabled-category origin success, missing/existing payer cases,
   retained static payer replacement, coalescing, cancellation and drain. A sibling
   permitted category must remain usable. Old `serve`/`warm`/`on_demand`/`search`
   keys reject; absent relay section stays origin-only; no agent override appears.

3. **P03 — Integration, handoff and retirement.** Complete combined validation,
   resolve findings, verify offline README managed/Tor/embedded examples and new
   defaults, unchanged method order, concise help and preserved discovery behavior.
   Confirm all old knobs are absent except deliberate rejection/historical-evidence
   tests; inspect references semantically, not by banning words such as "pricing"
   or "probe" that retained behavior still needs. Verify permanent documentation
   and AGENTS.md links, remove this completed plan and its index entry, and create
   no completed-plan archive. Review the exact cleanup SHA and final combined branch.

## Reviews, evidence and exit criteria

Independent design and clarity reviews passed during plan creation, with no
material findings remaining. P01/G1 passed through `05ee377b`; P02 implementation is in progress; material plan changes
or new evidence reopen affected design concerns before dependent work. Every implementation
commit receives an independent exact-SHA review of surrounding
consumers, intent, baseline and actual validation evidence before dependent work.
Address postcommit findings with `git commit --fixup=<introducing-SHA>` and review
each exact fixup; preserve unsquashed history through final integration review.

- **G1 after P01 — Pricing responsibility removal.** Review all prior plan commits
  and fixups through standalone/deployment/warming/qualification paths, actual
  payment challenge→validated hint→wallet selection, retained generic validation,
  catalog publication and old evidence readers. Exit: zero proactive endpoint
  pricing I/O or required phantom stage, intact passive/authored pricing and
  financial ownership, no new replacement workflow, and source-matched complexity.
- **G2 after P02 — Disclosure and payment boundaries.** Independently investigate
  category/network-consumer routing and financial/ownership concerns; divide work
  where independent questions warrant it, without imposing a reviewer count.
  Review every earlier commit/fixup and full chains, including P01 deletions,
  defaults, cache identity, mixed workflows, loader timing and relay recursion.
  Exit: truthful categories, no disabled-category relay spend or cross-category
  bypass, no payer/authority drift, accepted work retained, documentation accurate
  and source-matched complexity. Earlier approvals do not exempt prior code.
- **G3 after P03 cleanup — Final integration.** Review the exact cleanup SHA and
  complete combined branch/fixups, permanent contracts and actual test artifacts.
  Exit: no material findings, complete handoff, successful required validation,
  preserved user files/discovery fixes and no unperformed live qualification claim.

At each gate, analyze fixup causes, repeated assumptions and coverage gaps; decide
whether they require wider audit, regressions, simplification or design changes.
Later material changes reopen affected concerns; do not build on unresolved ones.
Keep reviewers read-only by default; coordinate any delegated edits explicitly.

Use the pinned CogC/CycC analyzer and compare against exact baseline `163a3e79`
before substantive Rust edits and at G1/G2/G3. Verify the reporter resolves that
full SHA, inventories deleted/relocated responsibilities and matches current source
hashes. Review warnings and unchanged high-risk payment/deployment/funding hotspots
in the complete call chains. Evaluate concrete simplifications, not score-only
helper extraction or merely claiming that responsibilities are necessary. Store
generated metrics/reports under ignored `target/`, not this plan or an archive.

Use focused actual-consumer regression tests, then appropriate broader coverage:
config/CLI/provider catalogs, startup/deployment/cache warming, discovery/help/import,
relay/network audit, payment hint/best-effort/ownership and qualification/live-runner
offline preparation/report tests. Verify nonzero executed counts and forbidden side
effects; remove obsolete probe-only tests while preserving their still-relevant
transport, publication, authority and ownership coverage on retained consumers.
Run formatting and applicable all-target Clippy. Finish the required application
check (`scripts/check.sh`, including its feature/lifecycle/compatibility components)
with the agreed baseline; focused corrections need not repeat unaffected suites.

Use `scripts/zcash.sh`, Python 3.11+ for development tooling and the pinned toolchain.
Choose socket-capable execution up front for socket fixtures; keep socket-free
checks in the normal sandbox. Freeze sources/executables during validation, avoid
competing builds and run feature configurations sequentially. Do not change pins,
trust policy or timers to pass tests. No real-wallet, live paid, public mainnet or
Docker consensus execution is part of this plan.
