# Maintainer disposition and decisions

This is working input for the [orchestration plan](review_orchestration.md), not
an additional implementation plan or authorization. The original report is
preserved verbatim below, including its historical recommendations and conclusions.
Those recommendations are source material, not instructions to execute. The
maintainer decisions here and the owning guides govern the selected work.

- Original reviewed revision: `3ceb88f2e41038c4f6ef6959494c2f501373f1cd`.
- Unannotated copy: commit `842b4c4bf11d2311720467fae4b0fcd2e66d4b7e`, file
  `docs/plans/x402_treazury_review.md`, SHA-256
  `64d44587525af5c24fa30514da00939a8f845e5479dda5a9d3fc35a62486913e`.
- Disposition baseline: `daabc401cedc169ed4e914b92fe276bb2876fd30`, including
  `main` at `a626b998` and the reviewed implementation/fixups through `d557d82a`.
  Original line numbers, counts and claims of verification belong to the old
  revision; they do not establish current behavior or current validation.

## Settled decisions

1. **S1:** Empty/unset cap values may mean no cap where the configuration permits
   them. An empty parsed cap emits WARN recommending explicit `none`/`off`.
   Existing defaults and required-setting rules remain unchanged; finite amount
   and qualification-budget parsers reject empty input without that misleading WARN.
2. **S3:** Newly signed x402 authority lasts at most five minutes (300 seconds),
   honoring a shorter provider lifetime. This is signed financial expiry, not an
   HTTP timeout. Preserve original accepted offers and actual journaled expiry;
   existing signatures are not rewritten and expiry grants no replay authority.
3. **S7:** Provider upstreams require HTTPS with full certificate-chain and hostname
   validation, including Tor routes. There is no insecure provider opt-in.
   Infrastructure HTTP contracts and local MCP/control serving remain separate.
4. **C1:** Preserve the conventional missing-token fallback and the existing
   independently disabled auth/Host-check settings. Emit both warnings when both
   protections are disabled; do not replace this decision with fail-closed startup.
5. **Financial boundaries:** Retain per-wallet structured-insufficient-funds traversal,
   first-positive-credit claim consumption, SDK-owned spendability and accepted-work
   ownership. No new ordinary ZEC spending budgets, credited-output floor, top-ups,
   whole-work deadlines or abandonment of accepted work. Randomized reserve bounds
   authorize nominal sizing only. Qualification's API budget is separate authority.
6. **Scope:** Orchestration's remaining focused plans plus portability own unfinished implementation.
   R9 receives a separate scoped plan under orchestration R0/R1. Broad cover shaping
   and discovery/indexer privacy changes are excluded, except tangible direct fixes.
   Other deferred observations are not silently resolved or authorized by this table.

## Finding disposition

**Addressed** means the accepted scope was implemented, not that every original
recommendation was adopted. **Planned** remains unfinished. **Partial** identifies
both completed and remaining scope. **Policy** records a deliberate choice, not
proof that a stated trade-off is absent. **Deferred** means no implementation was
selected here; historical details not freshly checked are explicitly identified.

| ID | Disposition | Decision, remaining work and evidence/owner |
| --- | --- | --- |
| S1 | Addressed / policy | Retain empty-as-unlimited with WARN and explicit `none`/`off` advice; defaults unchanged. [Parser](../../src/payment.rs), finite-budget guard and [warning regression](../../src/server/security_tests.rs). Implemented in `83dd3ab2` and its fixups. |
| S2 | Policy | Try each existing eligible wallet once only after recognized structured insufficient funds, with each authorization potentially chargeable. Do not add a traversal cap, on-chain precondition, allocation or funding wait. Advisory low-balance information cannot reopen a consumed initial funding claim. [Unsigned retry contract](../architecture.md#unsigned-x402-requests-and-post-retries), [best-effort tests](../../src/payment/best_effort_tests.rs). |
| S3 | Addressed | All registered static v1/v2 exact/upto and managed exact signing use the shorter lifetime or 300 seconds, preserving opaque accepted data and original liability. [Signing boundary](../../src/payment/lifetime.rs), [lifetime tests](../../src/payment/lifetime_tests.rs), managed offer tests and payment compatibility evidence. |
| S4 | Addressed in accepted scope | `47b30aa8` removes FAILED/REFUNDED-only replacement authority; fixups `fdb71bba` and `625d6d6b` cover treasury recovery and explicit SDK reconfirmation through cancellation/restart. Canonical confirmation moves active expiry to historical evidence, which cannot grant another deposit. `ba368fe6` plus documentation fixup `88213cab` cover fixed/ranged redraw, late archived SUCCESS/credit and two disclosed originals. Independent exact-SHA and combined G1/G2 reviews accepted the scoped offline evidence; full-suite fixture limits remain below. See [funding authority](../wallet-rotation.md#funding-and-source-safety), [Store regressions](../../src/rotation/store/funding.rs) and [lifecycle contracts](../../tests/LIFECYCLE.md). |
| S5 | Policy; trust assumption remains | Do not introduce or restore a configurable per-transfer ZEC cap or derive one from nominal USDC bounds. Merged `main` deliberately removed that setting and qualification ZEC budgets. Actual spendable treasury capital, accepted principal and network-fee safeguards govern. Provider USD valuation remains a trust assumption, not an independent price oracle or a newly fixed issue. [Financial policy](../../AGENTS.md#financial-and-execution-boundaries), [funding guide](../wallet-rotation.md#funding-and-source-safety). |
| S6 | Documentation addressed / policy | Documented compounded overhead/slippage (about 10.53% at 5%/5%). Do not impose a credited-output floor: nominal sizing is not guaranteed settlement output; first positive credit consumes the claim. [Funding guidance](../wallet-rotation.md#funding-and-source-safety). No claim that favorable/adverse market outcomes were eliminated. |
| S7 | Addressed | Reject provider HTTP before acquisition/signing; retain full TLS validation and immutable additional trust for private PKI. Infrastructure transports remain separately governed. [Network policy](../network-egress.md), [factory](../../src/network.rs), TLS success/wrong-host/untrusted controls. No certificate/hostname bypass. |
| C1 | Addressed warning coverage / policy | Retain missing conventional token fallback and independent auth/Host-check controls. Both WARNs are asserted together in [security tests](../../src/server/security_tests.rs). The report's fail-closed/refusal recommendations were not adopted. |
| C2 | Addressed in accepted scope | Standalone HTTP resolves and validates all loopback addresses before provider I/O, then binds captured addresses. Bearer scheme is case-insensitive; token comparison remains exact. CLI token option remains with an exposure WARN; env input is preferred. [Standalone path](../../src/main.rs), [auth](../../src/server/auth.rs), [server regression](../../tests/server.rs). |
| A1 | Planned | Protect all construction/expansion paths, including direct Value entrypoints, using justified shared accounting and stack-safe traversal. Parsing-only checks are insufficient; avoid imposing unrelated dynamic-import policy on authored catalogs. [Catalog safety/schema plan](catalog_safety_and_schema.md). |
| A2 | Planned | Distinguish an uncorroborated stale tip from genuine chain replacement; retain deep-reorg recovery and justified persisted earlier scan floors. A blanket return to ChainError or birthday clamp is not the selected fix. [Indexer/recovery plan](indexer_regression_recovery.md), [SDK contract](../zingo-integration.md). |
| A3 | Policy; availability trade-off retained | Do not add a whole-call deadline or arbitrary Retry-After ceiling. Preserve per-response inactivity, existing acquisition semantics and ownership of accepted work. Signed expiry does not cancel HTTP progress. This is not a claim that hostile progressing work cannot consume capacity. [Network timers](../network-egress.md#remaining-timers-and-budgets), [resource policy](../local-resource-limits.md). |
| A4 | Partly obsolete; remaining work planned | Both calculation paths already use embedded lazy Sapling parameters; the disk-file/missing-parameter claim is stale. Per-operation adapters still reparse when used, and synchronous CPU calculation remains on the serving runtime. [Proving plan](proving_execution.md) covers safe reuse and owned offload while preserving accepted principal/fee authority. |
| A5 | Partial | Immediate sanitized terminal funding-owner diagnostics now identify safe typed causes. Existing scoped decode/completion handling must not be described as absent. Shared Store failures can still terminate the funding lifecycle; no wholesale fault-scope redesign or automatic supervisor was added. [Funding owner](../../src/rotation/funding.rs), [scheduler](../../src/rotation/funding/scheduler.rs), owner diagnostic tests. |
| A6 | Partial | Source-owner panic recovery now emits INFO; retained-owner retry diagnostics remain DEBUG. No new retry timer/backoff or Store-worker supervisor was selected. Restarting financial ownership cannot be presumed safe merely because a task exited. [Funding owner](../../src/rotation/funding.rs), [accepted-work contract](../wallet-rotation.md#funding-and-source-safety). |
| A7 | Policy | Completed financial publication remains owned and retried rather than abandoned on cancellation/shutdown. Intermediate retriable failures do not automatically become WARN. No new command-queue redesign was selected. [Treasury owner](../../src/treasury/mod.rs), [ownership/reconciliation policy](../wallet-rotation.md#recovery-and-reconciliation-policy). |
| P1 | Deferred; excluded scope | Broad cover episode shaping is outside this work. Original timing claims are not requalified or marked fixed. [Cover policy](../cover-traffic.md); any future change needs its own concrete threat model and evidence. |
| P2 | Deferred; excluded scope | Same cover-shaping boundary as P1. No timing/anonymity guarantee is inferred from current fixtures or default-on settings. [Cover policy](../cover-traffic.md). |
| P3 | Deferred; excluded scope | Broad shared/per-wallet circuit timing changes are outside this work. Preserve existing network identities; investigate a tangible direct defect separately rather than add speculative scheduling/cache policy. [Network policy](../network-egress.md). |
| P4 | Deferred; excluded scope | Broad indexer-address privacy changes are outside this work. A2's chain-recovery plan does not implement address-query isolation. Original clustering claims are not marked fixed. [SDK integration](../zingo-integration.md). |
| P5 | Deferred; excluded scope | No new discovery identity split was selected. Original Coinbase isolation observation is not independently requalified by this disposition. [Agent-source network categories](../agent-sources.md), [network policy](../network-egress.md). |
| P6 | Addressed | Dirty SQLite preflight copies use a private temporary directory inside the state directory; clean read-only inspection and source DB/WAL preservation remain intact. [Preflight](../../src/rotation/store/preflight.rs) and its cleanup/byte-preservation regressions. This does not solve R6 snapshot retention. |
| R1 | Policy | First positive advisory USDC credit consumes the wallet's initial claim, including partial credit; low/reorged estimates do not reopen it. Ordinary payment selection remains advisory. Confirmation-gated readiness or a minimum target fraction was not adopted. [Best-effort rules](../wallet-rotation.md#best-effort-wallet-rules), [observation policy](../wallet-rotation.md#wallet-observation-policy). |
| R2 | Planned; original scope overstated | Mutation construction already runs in spawn_blocking and an aggregate document guard already exists. Remaining work is repeated build/parse, lookup cost and async readers sharing expensive mutation locking. No registration cap or identity normalization selected. [Registry-efficiency plan](registry_efficiency.md). |
| R3 | Deferred assessment | Description/control-character/raw-pricing concerns were not fixed by the selected changes. No new arbitrary description cap selected; inspect actual output contracts before choosing a focused transformation. A1 expansion protection is not a claim to solve agent-facing untrusted prose. [Tool documentation policy](../agent-sources.md#tool-documentation). |
| R4 | Policy; explicit endpoint trust retained | Explicit operation URL overrides remain supported, including public HTTPS hosts outside the base origin. Generic path-origin checks are not a blanket restriction on those overrides. Dynamic destinations retain public-URL guards and all providers now require HTTPS; these checks do not establish that a provider's chosen payee is trustworthy. No new provider-domain allowlist selected. [Import validation](../../src/discovery/import.rs), [provider configuration](../configuration.md). |
| R5 | Policy; cost assessment deferred | Metadata relay fallback is explicitly configurable and each paid authorization remains independently chargeable. No blanket ban on URL-probe fallback selected. Original worst-case request/buffering figures have not been freshly measured; assess a concrete probe optimization separately without weakening payment uncertainty. [Relay categories](../agent-sources.md), [source details](../../src/discovery/source_details.rs). |
| R6 | Deferred | Retained encrypted snapshots have financial provenance/recovery consumers. No snapshot-pruning implementation was selected or implied by P6. A future retention change must enumerate all required references and preserve accepted work and audit/recovery evidence; deleting rows or keeping only a hash is not automatically sufficient. [Store](../../src/rotation/store.rs), [recovery policy](../wallet-rotation.md#recovery-and-reconciliation-policy). |
| R7 | Policy; diagnostic cleanup addressed | Do not restore an arbitrary freshness margin. Actual accepted quote deadlines and signed consensus expiry govern; external arrival/settlement details remain a separate contract question. Obsolete diagnostic arm removed. [Timer inventory](../network-egress.md#remaining-timers-and-budgets), [funding contract](../wallet-rotation.md#funding-and-source-safety). |
| R8 | Planned; composed-body claim needs qualification | Path/query collisions, unsupported cookie routing and truthful media/serialization need focused coverage. Composed-body support already exists in parts: fix demonstrated gaps, preserve alternatives, do not blindly flatten exclusive schemas. [Catalog safety/schema plan](catalog_safety_and_schema.md). |
| R9 | Planned; implementation pending | Ordinary regex compilation and HTTP cache-lock construction costs remain. Re-audit broad status consumers rather than repeat the historical 10,000-payment claim as current fact. New qualification admission scans case/run/global history, parsing payment JSON under the write transaction; that aggravation is confirmed without a latency measurement. [Qualification budget](../../src/qualification/budget.rs), [admission](../../src/qualification/payment.rs), [focused R9 plan](hot_path_efficiency.md), orchestration R0/R1 and the [R2 scope boundary](registry_efficiency.md). No cached monetary ledger, caps or pruning selected. |
| R10 | Original premise not accepted | The implementation compares the stored transaction's computed txid with the original signed transaction's computed txid, rather than merely comparing the lookup key. Preserve ZIP-244 effects identity and original disclosed bytes. No removal selected on the report's premise. [SDK confirmation](../../src/treasury/send.rs), [identity/recovery contract](../zingo-integration.md). |
| B1 | Deferred | CI and automated advisory coverage were not implemented here; existing vendor verifiers and maintained advisory notes are not equivalent to CI. Recheck current versions/advisories and intended native/MSRV scope before selecting pipeline work. Do not infer vulnerability resolution or authorize a compiler/platform matrix from this report. [Development](../development.md), [vendor provenance](../../vendor/README.md). |
| B2 | Planned with A2 | Independent full-patch/recovery review belongs to the [indexer/recovery plan](indexer_regression_recovery.md). Upstream submission is separate authority; retain vendor provenance and genuine reorg coverage. Existing application review is not a claim of upstream approval. |
| B3 | Policy plus plan review work | Mandatory complexity review and simplification at child architecture gates remain; no numerical complexity budget or automatic helper extraction. Review unchanged high-risk financial chains as well as changed functions. [Complexity discipline](../../tests/COMPLEXITY.md#review-discipline), [orchestration gates](review_orchestration.md). No blanket refactor completed. |
| B4 | Deferred broader cleanup | S4 reviewed archive/reconfirmation consumers and distinguished active expiry authority from retained historical evidence. Superficially similar SQL can carry distinct ownership/history behavior. No general SQL-format/archive-helper rewrite completed, and historical counts are not newly verified. [Recovery policy](../wallet-rotation.md#recovery-and-reconciliation-policy), [Store funding](../../src/rotation/store/funding.rs). |
| B5 | Partial | Known obsolete diagnostic arms removed and new owner diagnostics use typed safe context. Broader string-based classification remains; no repository-wide conversion claimed. Prefer typed causes when changing owning paths without logging raw upstream prose. [Funding diagnostics](../../src/rotation/funding.rs), [logging contract](../development.md#logging-and-diagnostics). |
| B6 | Deferred assessment / policy | Attribute counts alone do not establish a production defect. No blanket test-hook-to-trait rewrite selected; inspect changed financial boundaries and their production/test equivalence through owning reviews and production compiler evidence. Original hook inventory is not newly verified. [Testing policy](../testing.md). |
| B7 | Deferred | Public-surface pruning was not implemented here. Re-audit actual library, integration-test and tooling consumers before narrowing visibility; historical counts are not current acceptance evidence. [Library surface](../../src/lib.rs), [development](../development.md). |
| B8 | Planned | Shared private-storage contracts require equivalent Linux/macOS/Windows protection, native evidence and explicit refusal where unavailable. Compilation alone is insufficient. Include source/cache stores, financial journals and version-2 qualification authority without silently upgrading historical registries. [Portability plan](portability_fixes.md), [permanent policy](../development.md#portability). |
| B9 | Deferred | No shared deployment-loader refactor completed. Inspect current show/load/warm/bootstrap ordering and failure contracts before selecting a simplification; the original literal/field counts are not reverified. [Deployment](../../src/deployment.rs), [configuration](../configuration.md). |
| B10 | Deferred item-by-item | This bundles distinct build, dependency, locking, key-storage and panic observations. No blanket remediation or fresh inventory claimed. Dependency/toolchain changes retain the existing pin/reproducibility policy. Release arithmetic, lock replacement and other items require targeted assessment and tests in their owning area. [Development](../development.md), [reproducible builds](../reproducible-builds.md), [wallet CLI](../wallet-cli.md). |

## Validation and final annotation

Implementation evidence at the disposition baseline is staged: 32 focused library
regressions and 30 compatibility tests passed; the production/all-target warning
gate passed, with affected server-target revalidation after its final test-only
fixup. Source-matched complexity completed with no exclusions or regression
warnings. The full default/proving/lifecycle/codec suites were not run for that rebase;
no live qualification is claimed. macOS emitted a compact-unwind size warning for
the large test binary, separately from the clean source warning gate. These
results neither close planned findings nor establish the original report's
unverified claims.

At each owning integration gate, update the affected rows with exact implementation
and independently reviewed fixup SHAs, concrete test evidence and remaining limits.
At final orchestration review reconcile all 42 IDs against the original report and
settled decisions. A deferred or policy row needs an explicit disposition, not a
fabricated fix; required planned work cannot be closed by annotation alone.
Preserve the original body below while this working reference remains active.
Before completion, transfer enduring rules to owning guides and retain concrete
unfinished work in focused plans, then remove this completed working material and
its links under the [documentation policy](../README.md). Independently review the
cleanup SHA and final combined branch; never remove financial evidence.

Replacement-authority acceptance through `625d6d6b` includes 51 Store funding,
11 treasury recovery, 30 compatibility, 25 funding integration, 17 rotation and
2 treasury integration tests; four proving tests and 35 lifecycle tests passed
using a focused continuation, as did the address-codec test.
Production and all-target compiler/Clippy warning gates passed; source-matched
complexity had no exclusions or production regression warnings. The large macOS
test harness still emits its known compact-unwind linker size warning.

This is **not** a green full default, managed or lifecycle stage. Twenty unrelated
failures and a stalled deferred-startup fixture were attributed to unchanged O0
test/consumer sources (33 compared files), including TLS trust/key setup, HTTP
provider fixtures, URI/error assertions and retained registry ownership. The
baseline executable was not rebuilt. Managed passed 43 tests but failed three;
lifecycle carried 34 passes plus the corrected reconfirmation test, while its
unchanged HTTP worker fixture failed before payment/rotation assertions. Two
milestone-invalidated permissive recovery tests were corrected in the F1 fixups;
they are not part of the unrelated-failure count. The
[orchestration validation follow-up](review_orchestration.md#unfinished-validation-fixtures)
retains the unfinished work and unvalidated assertions. No live-funded, consensus
or Tor qualification was run.

---

# x402\_treazury Code Review

Nov 2, 2026 · @B

## Summary

The core money logic is careful, and no direct fund-loss bug was found. The main risks are places where the code trusts providers, the swap service or indexers without a local limit, plus settings that silently switch safety off.

- **Repository:** [mikeperry-tor/x402\_treazury](https://github.com/mikeperry-tor/x402_treazury), branch `main`
- **Commit reviewed:** `3ceb88f2e41038c4f6ef6959494c2f501373f1cd` (2026-11-01 17:30 UTC, "Use rust nightly for conditional coverage benchmark for pruning plan")
- **Scope:** about 49k lines of production Rust in `src/`, plus the vendored patches to reqwest, hyper, h2 and pepper-sync. Tests were read only to understand intent.
- **Review type:** static reading only. Nothing was built or run.

Findings are grouped by area. Each has a severity, a code location and a suggested fix. Items marked **Verified** were checked against the code by the review lead; the rest come from area reviewers and should be confirmed before acting.

## Spending safety

Seven findings can make the process sign or spend more than the operator intended. Items S1–S5 were verified against the code.

| # | Finding | Where | Severity | Status | Suggested fix |
| --- | --- | --- | --- | --- | --- |
| S1 | An empty spending cap means unlimited. `SpendPolicy::dollars` treats `""` like `"none"`/`"off"`. An exported but empty `X402_MAX_PRICE_USD=` overrides the `1.00` default, and TOML `max_api_payment_usdc = ""` passes validation. | `src/payment.rs:192`; `src/main.rs:324-326`; `src/rotation/config.rs:204-205` | High | Verified | Reject the empty string; accept only explicit `none`/`off`. |
| S2 | A provider can collect signed authorizations from every wallet in a pool. A bare 402 with `payment-required` = `{"error":"insufficient_funds","accepts":[]}` (no settlement receipt) makes the code re-sign with the next wallet until the pool is exhausted. Each wallet is also marked low-balance, which triggers refunding. | `src/payment.rs:795-818`, `:1385-1424` | High | Verified | Require the receipt form, check on-chain balance before moving on, or allow at most one extra wallet. |
| S3 | The provider chooses how long a signed authorization stays valid. `maxTimeoutSeconds` is bounded only by `i64::MAX - now`. Long-lived unsettled authorizations also stay `POSSIBLY_SUBMITTED` and can block retirement. | `src/rotation/manager.rs:633-639`, `:706-716`; `src/rotation/base.rs:720` | Medium | Verified | Enforce a local maximum (minutes to one hour) before signing. |
| S4 | A provider `FAILED` or `REFUNDED` status alone allows a replacement Zcash deposit. This contradicts `docs/wallet-rotation.md:676` ("a refund alone cannot authorize a replacement deposit"). If the solver delivers late, the wallet is funded twice. | `src/rotation/store/funding.rs:405-418`; `src/treasury/recovery.rs:309` | Medium | Verified | Require corroborating evidence (expiry or chain facts) before re-authorizing. |
| S5 | No default ZEC cap per swap. Unset `max_funding_spend_zec` becomes `i64::MAX`. The only price check compares the swap service's own `amountInUsd` with its own `amountOut`. | `src/rotation/funding.rs:472-476`; `src/rotation/near.rs:787-816` | Medium | Verified | Require a cap, or derive one from the USDC target. |
| S6 | On the EXACT\_INPUT path, slippage stacks on top of the overhead cap (about 10% worst case with defaults, not 5%). The executable `minAmountOut` is never compared to the funding target, so a wallet can be published under-funded. | `src/rotation/near.rs:714-728`, `:768-779`; `src/rotation/store/funding.rs:435-446` | Medium | Not re-checked | Check `minAmountOut` against the target; document the combined bound. |
| S7 | Paid requests may use cleartext `http://`, including through Tor exits, which could rewrite `payTo`. Only `public_only` sources require https. | `src/network.rs:916-919`; `src/main.rs:240` | Medium | Not re-checked | Require https for any request that may be signed, or an explicit opt-in. |

## Configuration that fails open

Two serving paths can expose a payment-signing endpoint without the operator choosing to. Both were verified.

| # | Finding | Where | Severity | Status | Suggested fix |
| --- | --- | --- | --- | --- | --- |
| C1 | Listener auth turns off when the token variable is missing. If `bearer_token_env` is the conventional `X402_TREAZURY_MCP_TOKEN` and it is not set, `token()` returns `None` and the listener serves unauthenticated, with only a WARN. Every shipped example uses that name. `auth = false` together with `disable_host_check = true` is also accepted, which allows DNS rebinding. | `src/deployment.rs:123-128`; `src/server.rs:~374` | Medium | Verified | Fail closed unless `auth = false` is set explicitly; refuse `disable_host_check` without auth. |
| C2 | Standalone `serve --transport http` binds any host with no loopback check, while deployment mode requires loopback. `--host 0.0.0.0 --no-auth` exposes signing to the LAN; with auth, the token crosses the LAN in cleartext. `--bearer-token` puts the secret in `ps` and shell history. The auth scheme match is case-sensitive (`Bearer` only). | `src/main.rs:273`; `src/cli.rs:139`, `:143`; `src/server/auth.rs:72-77` | Medium | Verified (bind) | Apply the deployment loopback rule; deprecate the CLI token flag in favour of env or file input. |

## Availability and denial of service

A single hostile spec, indexer or provider can crash the process, stall paid calls or block funding for days.

| # | Finding | Where | Severity | Status | Suggested fix |
| --- | --- | --- | --- | --- | --- |
| A1 | `$ref` expansion has no depth or size limit for static and relay-supplied catalogs. A 200k-link chain overflows the stack (process abort); nested `allOf` references expand exponentially. The `inspect()` guard (depth 64, 500k nodes, 64 MiB) runs only on the agent-import path. | `src/catalog.rs:966-990`; `src/discovery/import.rs:149-193` | Medium | Verified | Run the same complexity guard inside `catalog::parse_document` or `build_tools`. |
| A2 | The pepper-sync reorg patch turns an implausible indexer tip into a full wallet clear and rescan from Sapling activation (mainnet height 419,200). Upstream returned `ChainError`. A tip 100+ blocks behind, from a lagging or hostile indexer, blocks funding for hours to days. `scan_birthday` then keeps the lowered floor permanently. | `vendor/pepper-sync/src/sync.rs:991`, `:1045-1105`, `:2414-2439` | Medium-High | Verified | Keep upstream's fail-closed error, or reject tip regressions against the last observed height in `sync_inner`. Don't persist the activation floor. |
| A3 | `Retry-After` is followed with no cap (any u64 or HTTP-date), and there is no overall per-call deadline. Accepted work keeps running after client cancel, and each call holds one of 128 process-wide permits. About 128 stuck calls reject every paid call on every listener. The patched body timer, renewed by any DATA frame, allows the same with a slow trickle. | `src/network.rs:743-790`; `src/network/acquisition.rs:147-149`; `src/payment/ownership.rs:7` | Medium | Not re-checked | Cap `Retry-After` (60–300 s) and add a per-call deadline. |
| A4 | Zcash proving runs on the shared serving runtime while holding the wallet write lock. Shielding also reloads and hashes about 50 MB of parameters from `~/.zcash-params` on every call, and fails silently if they are missing. | `src/treasury/send.rs:248-250`; `src/treasury/refunds.rs:184`, `:212`; `src/treasury/owned_transparent.rs:77` | Medium | Verified | Use `spawn_blocking`; reuse a static embedded `LazySaplingProver`; log shielding failures. |
| A5 | One store error ends the funding worker, and its drop guard also stops the treasury owner. Serving logs only "restart required"; the cause appears at shutdown. A per-operation decode error takes down all funding. | `src/rotation/funding/scheduler.rs:470-476`, `:583-601`; `src/rotation/funding.rs:164`; `src/deployment.rs:1893-1997` | Medium | Not re-checked | Scope per-operation faults; log the error when the worker stops. |
| A6 | A panic in a funding source step reruns every 5 s in a loop, logged at DEBUG. A panic in the store worker kills it for the process lifetime with no supervision. | `src/rotation/funding/scheduler.rs:770`; `src/rotation/store.rs:1166-1176` | Low | Not re-checked | Back off after panics; supervise the store worker and trigger shutdown. |
| A7 | Shutdown retries publication forever, ignoring cancellation, while the command loop stops draining its queue of 8. | `src/treasury/mod.rs:563` | Low | Not re-checked | Log at WARN after the first failure; honour cancellation. |

## Privacy

Cover traffic gives less protection than its default-on status suggests, and several request patterns let observers link wallets. None of these were re-checked by the review lead.

| # | Finding | Where | Severity | Suggested fix |
| --- | --- | --- | --- | --- |
| P1 | Cover stops under sustained load. Each episode's deadline is fixed at first call (default 10 s) and later calls don't extend it. While calls overlap, `calls` never reaches 0, so no fresh episode starts. Cover vanishes when the agent is busiest. | `src/cover/registry.rs:154`; `src/cover/episode.rs:30`; `src/cover/runtime.rs:563` | Medium | Anchor episodes to final response headers; allow a fresh episode while one is exhausted. |
| P2 | Cover timing marks real responses. Cover is paused until response headers, then the tail is clamped to the call-start deadline. Calls slower than about 10 s get no cover, failed calls end cover immediately, `start_delay` has no effect, and each new owner starts with a fixed `bytes=0-1023` request. | `src/cover/runtime.rs:153-160`, `:575-580`, `:759-761`; `src/cover/episode.rs:55` | Medium | Decouple timing from real-call events; state the limited threat model where operators see it. |
| P3 | Per-wallet Tor circuits are sandwiched between requests on one shared circuit: Base header reads and `/v0/tokens` run on the discovery circuit just before each wallet's `eth_call` or `/v0/quote`. Pinning to the exact block hash makes the link stronger. | `src/rotation/base.rs:614-631`, `:813-831`; `src/rotation/funding.rs:752-756` | Medium | Separate or randomize timing between shared and per-wallet circuits; cache the token list. |
| P4 | Zcash sync queries every transparent address (plus gap-limit addresses) over one `IsolationId::treasury` connection, so the indexer can cluster legacy refund addresses that submissions deliberately isolate. | `vendor/pepper-sync/src/sync/transparent.rs:63`, `:134` | Medium | Narrow discovery scopes or query each address under its own isolation. |
| P5 | Coinbase directory searches and URL probes use the wallet's EVM isolation identity, so search keywords share a circuit with paid calls. | `src/discovery/coinbase.rs:141` | Low | Offer a separate discovery identity. |
| P6 | After an unclean stop, the preflight check copies the whole DB and WAL to a `$TMPDIR` directory (mode 0700). Plaintext metadata (addresses, payees, amounts, pool names) leaves the state directory. | `src/rotation/store/preflight.rs:66-72` | Low | Use `tempdir_in(state_dir)`. |

## Correctness and performance

The biggest correctness gap is an unverified balance read driving an irreversible state change. The biggest performance cost is a full, quadratic registry rebuild on every registration.

| # | Finding | Where | Severity | Status | Suggested fix |
| --- | --- | --- | --- | --- | --- |
| R1 | One unconfirmed `eth_call … "latest"` balance read permanently uses up a wallet's funding claim and moves it to READY. Dust, a reorg or a lying fallback RPC can trigger it. The confirmation-depth path `record_verified_credit` has no production caller. | `src/rotation/store/balances.rs:95-117`; `src/rotation/store/evidence.rs:1240-1247` | Medium | Verified | Require verified, confirmation-depth evidence (or a minimum fraction of target) before `record_initial_credit`. |
| R2 | Each agent registration rebuilds the whole registry under a `std::sync::Mutex` that async code also locks. Specs are parsed and built about three times per commit. Nested loops are quadratic: `matches_operation_tags` runs even with no tag filters, plus linear name-collision and help-reuse lookups. There is no cap on registrations (query-string variants count as distinct). | `src/discovery/mod.rs:360-448`, `:708-866`; `src/catalog.rs:1193-1202` | Medium | Not re-checked | Cap registrations; cache built records by (id, revision); use maps; skip tag matching when unfiltered; never lock from async code. |
| R3 | Provider descriptions reach agent-visible tools with no length cap or control-character stripping. Unrecognized pricing shapes are appended as raw JSON. Registrations persist across restarts. | `src/catalog.rs:244`, `:1294`, `:1656-1690`; `src/discovery/import.rs:422-431` | Medium | Not re-checked | Apply a default description cap to imports; strip control characters; omit raw pricing JSON. |
| R4 | A remote spec using `operation_url_extension` can send paid calls to any public HTTPS host. | `src/catalog.rs:1733-1755`; `src/discovery/import.rs:488`; `providers/lonestar.toml` | Low-Medium | Not re-checked | Restrict request URLs to operator-listed origins or the provider's domain. |
| R5 | One `provider_details_remote` call can trigger up to 24 paid relay fetches (3 bases × 8 paths) and buffer about 128 MB. | `src/discovery/source_details.rs:16-25`, `:96-113`, `:202` | Low-Medium | Not re-checked | Disable relay fallback for speculative probes. |
| R6 | Every funding or shielding operation pins a full encrypted wallet snapshot (possibly megabytes) forever, because `outgoing` rows are never deleted. | `src/rotation/store.rs:599`; `src/treasury/mod.rs:119-129` | Medium | Not re-checked | Prune snapshots for resolved operations, keeping a hash for audit. |
| R7 | No freshness margin before broadcasting against a quote deadline (default 7200 s). A dead "quote deadline too close" match arm suggests a margin check was removed. | `src/rotation/funding.rs:309-313`, `:404`, `:843` | Low-Medium | Not re-checked | Re-quote when the remaining window is below a margin. |
| R8 | OpenAPI conversion gaps: a query parameter overwrites a path parameter of the same name; cookie parameters are sent as query; `allOf`/`oneOf` bodies produce no input fields; non-JSON bodies are sent as JSON. | `src/catalog.rs:1551-1588` | Low | Not re-checked | Suffix collisions; reject cookies; flatten composed bodies. |
| R9 | Hot-path overheads: regexes recompiled per call (`slug()`, `money()`); HTTP clients built under a global mutex; qualification-mode status queries scan full history on every paid request and stop working after 10,000 payments. | `src/catalog.rs:395`, `:1244`; `src/network.rs:484-491`; `src/rotation/store/reporting.rs:150-231`; `src/qualification.rs:166-181` | Low | Not re-checked | Use `OnceLock` regexes; build clients outside the lock; bound status scans. |
| R10 | `confirm_sdk_transaction` checks a txid against the record it was just fetched by, so it can never fail. Its comment claims a ZIP-244 comparison that isn't done. | `src/treasury/send.rs:600`, `:614` | Low | Not re-checked | Compare something meaningful, or remove it. |

## Best practices and maintainability

The project has no CI, and the financial code is dense enough that review is the main bottleneck.

| # | Finding | Evidence | Severity | Suggested fix |
| --- | --- | --- | --- | --- |
| B1 | No CI and no automated security-advisory scanning. The four vendored crates are path dependencies with no `source` in `Cargo.lock`, so `cargo audit` and `cargo deny` can't see them. RUSTSEC-2026-0387 (crypto-bigint timing, in the EVM signing path) is tracked by hand. | No `.github/` or `.gitlab-ci.yml` (verified); `docs/development.md:561-600` | Medium | Add a Linux CI job: fmt, clippy `-D warnings`, `vendor/verify*.py`, tests, `cargo deny`, an MSRV 1.94.1 build, and an advisory check for the vendored versions. |
| B2 | The 668-line `pepper-sync-reorg.patch` is financial-correctness logic that upstream never reviewed (see A2). | `vendor/pepper-sync-reorg.patch` | Medium | Prioritize independent review or upstreaming. |
| B3 | Long functions in payment and funding paths: `shield_address` \~246 lines, `execute_response_owned` \~245, `prepare_with_observation` \~242, `serve` \~203, `initialize_wallets_mode` \~200, `maintenance` \~186, `dispatch_pending` \~181, `source_step` \~179. The complexity tool is advisory only. | `src/treasury/refunds.rs:69`; `src/payment.rs:652`; `src/treasury/send.rs:73`; `scripts/complexity.py:156` | Medium | Split these functions; enforce a complexity budget for financial modules. |
| B4 | SQL is hard to audit: statements of 300–600+ characters on one line; the 14-column `funding_recovery` archive INSERT is pasted five times with subtle differences (`OR IGNORE` vs plain, `attempts=0` reset in only one copy); read-named methods write. | `src/rotation/store/funding.rs:406`, `:1088`, `:1186`, `:1239`, `:1295`; `src/rotation/store/expiry.rs:68` | Medium | Move SQL into named constants or files; share one archive helper. |
| B5 | Errors are classified by matching message strings, and several arms are already dead (`"quote deadline too close"`, `"invalid transparent address"`, `"receipt log limit exceeded"`). | `src/rotation/funding.rs:843-849`, `:980-1012`; `src/rotation/base.rs:88`; `src/cover/runtime.rs:831-846` | Low | Classify by downcasting typed errors. |
| B6 | 312 `#[cfg(test)]` attributes in non-test files, including hooks inside financial state machines, so test and release builds run different code. | `src/rotation/funding/scheduler.rs:533`, `:566`; `src/rotation/manager/maintenance.rs:72` | Low-Medium | Replace hooks with injected traits. |
| B7 | Wide public surface: 20 `pub mod`s and 787 `pub` items in a `publish = false` crate. `pub` items escape dead-code warnings, which undercuts the existing pruning plan. | `src/lib.rs` | Low-Medium | Narrow to `pub(crate)`; enable `unreachable_pub`. |
| B8 | File permissions are silently skipped on non-Unix (`#[cfg(not(unix))] fn private_options(..) {}`), against AGENTS.md's "no silent no-ops" rule. Owner-only file handling is reimplemented in at least six places. | `src/rotation/store.rs:238-239`, `:420-421`; `src/http_cache.rs:640`; `src/discovery/store.rs:44-91` | Medium (latent) | One secure-fs module that fails closed on non-Unix. |
| B9 | The cross-config load pipeline and the 14-field `Deployment` literal are copied across show, load, warm and bootstrap paths; two copies already differ in order. | `src/deployment.rs:~420`, `:~603`, `:631`; `src/deployment/cache_warm.rs:~35`, `:108`; `src/deployment/bootstrap.rs:~80`, `:94` | Low | Share one loader. |
| B10 | Smaller items: no `[profile.release]` with `overflow-checks = true` (verified); declared MSRV 1.94.1 is never built (toolchain pins 1.99.1); `fs2` (no release since 2018) can be replaced by std file locks; `=` pins on pre-release Zcash, rmcp and x402 crates block patch fixes; `build.rs` re-hashes about 220k lines on every edit; release builds from `git archive` report `source_dirty=true`; the wallet key file sits beside the encrypted state; \~200 `unwrap()` and \~105 `expect()` in production code. | `Cargo.toml`; `rust-toolchain.toml`; `build.rs:46-70`; `src/rotation/config.rs:409` | Low | Address opportunistically. |

The history is 757 commits in one month from one author, most likely AI-assisted. That fits a recurring pattern in this review: docs and comments describe intended behaviour that the code doesn't always follow (S4, R1, R10).

## Strengths

Much of the hard plumbing is done well, which is why most findings are about limits and policy rather than mechanics.

- **Exact money arithmetic.** USDC and zatoshi amounts use U256/u64 with `checked_*` operations; no floating point for amounts.
- **Journal before disclosure.** Authorizations, broadcasts and calculations are persisted before any external side effect. No double-broadcast window after a crash was found in the treasury or store.
- **Single-writer store.** An exclusive owner lock plus one serialized SQLite worker removes write races. No SQL injection, and no blocking SQLite calls on async workers.
- **Centralized egress.** One `reqwest` client factory with redirects and library retries off. Tor mode uses `socks5h` with per-scope isolation and no direct fallback. Direct `public_only` mode checks every DNS answer at connect time.
- **Strict untrusted-input checks.** Agent-supplied URLs must be public HTTPS with exact origin checks; path and header encoding is safe; reserved headers are blocked.
- **Strict challenge and receipt validation.** Managed-wallet offers check `payTo`, amount, asset, network and the EIP-712 domain. NEAR quote request/response echo binding and Base receipt checks are thorough.
- **Encryption at rest.** ChaCha20-Poly1305 with per-record AAD, 0600 key files opened with `O_NOFOLLOW`, and `Zeroizing` buffers.
- **Careful qualification ledger.** Budgets and funding permits are reserved atomically under IMMEDIATE transactions.
- **Disciplined vendoring.** Hash-verified sources, reverse-apply checks and an upstream retirement ledger with linked issues.
- **Testing and docs.** Tests outnumber production code about 3:1, and the docs are candid about what is and isn't qualified.

## Recommended fix order and issues

Most of the top fixes are small and local. Suggested order:

1. Reject empty spending caps and fail closed on a missing auth token (S1, C1).
2. Limit wallet traversal on "insufficient funds", cap authorization lifetime, and require https before signing (S2, S3, S7).
3. Require corroboration before a replacement deposit, set a default ZEC cap, and check `minAmountOut` against the target (S4–S6).
4. Apply the complexity guard on every spec-loading path and cap `Retry-After` (A1, A3).
5. Restore fail-closed handling of a lagging chain tip in the pepper-sync patch (A2).
6. Add CI with clippy, tests, vendor verification and advisory scanning (B1).

The findings map to 24 issues, one per fix. Merging S4–S6, C1–C2, S2–S3 and R2 with R9 gives about 18. S1–S7 describe exploitable spending flaws, so they may suit a private security advisory rather than public issues.

| # | Proposed issue | Findings |
| --- | --- | --- |
| 1 | Empty spending cap means unlimited | S1 |
| 2 | "Insufficient funds" reply re-signs with every wallet | S2 |
| 3 | No local limit on authorization lifetime | S3 |
| 4 | Provider FAILED/REFUNDED status allows a replacement deposit | S4 |
| 5 | No default ZEC cap; price check trusts provider USD | S5 |
| 6 | Slippage stacks on overhead; quote not checked against target | S6 |
| 7 | Paid requests allowed over cleartext http | S7 |
| 8 | Auth turns off when the token variable is missing | C1 |
| 9 | Standalone serving lacks loopback check; token in argv | C2 |
| 10 | Unbounded `$ref` expansion can crash the process | A1 |
| 11 | Reorg patch lets a lagging tip force a full rescan | A2, B2 |
| 12 | Uncapped `Retry-After` and no per-call deadline | A3 |
| 13 | Proving blocks the serving runtime; parameters reloaded | A4 |
| 14 | One store error halts all funding without logging the cause | A5, A6, A7 |
| 15 | Cover traffic stops under load and marks real responses | P1, P2 |
| 16 | Tor circuit linkability across wallets, sync and discovery | P3, P4, P5 |
| 17 | Registry rebuild is quadratic, uncapped and locks from async | R2, R5 |
| 18 | Unconfirmed balance read consumes the funding claim | R1 |
| 19 | Wallet snapshots retained forever | R6, P6 |
| 20 | Hot-path regex, client-build and status-scan overheads | R9 |
| 21 | Add CI and advisory scanning | B1 |
| 22 | Non-Unix file permissions silently skipped | B8 |
| 23 | Readability: long functions, one-line SQL, string-matched errors, test hooks, `pub` surface | B3–B7, B9, R10 |
| 24 | Small hygiene: overflow checks, MSRV, `fs2`, pins, build identity | B10 |

R3, R4, R7 and R8 are not yet in this list. Filing them separately would make 28 issues; filing every low-severity note from the area reviews would make about 35–40.

## Method and limitations

This is a static review: nothing was compiled, tested or run, because the review sandbox did not allow running the repository's build scripts.

- **Coverage:** seven parallel reviewers each read one area line by line. The areas were the rotation store, the rotation funding and swap path, the Zcash treasury, payment and network, discovery and catalog, deployment, qualification and cover traffic, and project-wide practices.
- **Verification:** the review lead re-read the code for S1–S5, C1, C2 (bind), A1, A2, A4, R1, B1 and the missing release profile in B10. Everything else is the area reviewer's reading and should be confirmed before acting.
- **Not covered:** runtime behaviour, `cargo clippy` and test results, dependency advisory scans, and tests or fixtures beyond understanding intent.
- **Line numbers** refer to commit `3ceb88f`. A `~` marks an approximate location.
