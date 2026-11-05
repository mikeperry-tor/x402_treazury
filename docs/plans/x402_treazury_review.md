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
