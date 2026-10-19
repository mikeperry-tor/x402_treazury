# Local resource limits

Avoid creating additional limits. Arbitrary limits are largely considered
reliability hazards: they can reject valid actions or delay useful work for reasons
unrelated to the external service or blockchain. This inventory tracks existing
limits so operators can investigate unexpected failures. Listing a limit does not
justify retaining it or adding another one. Review the concrete resource or
correctness invariant before enforcing a limit; prefer removing redundant policy.

This guide covers foreground ownership, API caches, response transport and treasury
maintenance. The [network inventory](network-egress.md#remaining-timers-and-budgets)
tracks other timers, queues and polling policies.

| Resource or cadence | Current value | Effect and control |
| --- | --- | --- |
| Foreground work owners | 128 process-wide, hardcoded | MCP tool calls and direct paid-client workflows acquire a permit before retained work starts. Saturation rejects immediately rather than queuing, including cheap or unsigned tool calls. Listing tools is outside this cap. Accepted work retains its permit through response construction despite waiter cancellation; nested payment work reuses it. Intended to bound retained tasks and response memory. |
| Runtime API price hints | 65,535 entries per process; 24 hours from unsigned response receipt | Fixed-size entries guide wallet selection only. Capacity pressure evicts the oldest receipt; expiry, eviction and restart lose the hint without failing a call, probing or changing payment authority. Reads and late validation do not renew lifetime. Hardcoded capacity and lifetime. |
| API catalog disk lifetime | 24 hours by default | `catalog_cache_ttl_seconds` overrides the default per provider/source; zero disables catalog disk caching. After expiry, the next load revalidates or fetches normally. Refresh failure does not serve the expired document. Already published catalogs remain usable; expiry does not trigger background I/O. |
| API response bodies | 16 MiB by default | `max_response_bytes` bounds in-memory collection of API bodies, payment challenges and ordinary errors. Exceeding it rejects the response without partial success; a signed attempt may already have charged. |
| Help documents | 4 MiB by default | `max_help_bytes` bounds help downloads; an oversized document fails without partial success. |
| Remote OpenAPI documents | 32 MiB by default | `max_spec_bytes` bounds remote catalog downloads; local operator-authored files are unaffected. |
| Dynamic source import and registry | 512 dynamic tools per eager endpoint by default; on-demand endpoints exempt | `source_management.max_tools_per_server` accepts 1–10,000. No registration-count or per-source tool-count quota. Remaining document, schema, name, registry and import-capacity bounds are listed in [agent sources](agent-sources.md#limits-and-network-policy); rejections emit WARN without private content. |
| Connection establishment | 15 seconds direct; 120 seconds Tor | `connect_timeout_seconds`, configurable from 1 through 300 seconds, covers connection/TLS/SOCKS establishment. |
| Response read inactivity | 60 seconds by default | `read_timeout_seconds`; provider/source overrides inherit the network default when omitted. Network/source-management values range from 1 through 86,400 seconds; provider values may be fractional. Response headers and body reads are covered, with no whole-response deadline. Nonempty body progress renews only that response's timer. |
| Treasury observation and local recovery | At least 60 seconds after scan completion, hardcoded; scan errors back off to 300 seconds. Publication and SDK reconstruction retry independently after 60 seconds | Available treasury startup and explicit sync request one pass; recurring scans need current enabled funding demand or validated pending source/shielding work. Idle treasury does not scan for incoming funds. Busy/locked read-only demand acquisition uses a 60-second local reevaluation wake without authorizing a scan or stopping accepted commands; commands and notifications may recheck those read-only facts sooner. Waiting operations share successful current-owner scans without intervening tip probes; actual financial validation/expiry proofs remain separate. Publication/invalid input ownership alone does not demand network scans; owned shutdown retains identical local publication retries without recalculating or submitting. Successful needed scans can trigger owned-transparent shielding under existing authority. See [wallet observation policy](wallet-rotation.md#wallet-observation-policy); scheduling is not a work deadline or authority expiry. |
| Requested USDC refresh | At least 60 seconds after acquisition completion, hardcoded; error backoff up to 300 seconds | Only recognized insufficient-funds requests recur, with a finite background startup discovery pass. A successful zero finishes requested work; idle wallets issue no recurring Base reads. Store/wallet ownership coalesces acquisitions and retains pending requests arriving during reads. See [wallet observation policy](wallet-rotation.md#wallet-observation-policy). |
| Initial USDC destination and credit observation | One destination discovery read after a valid spendable SDK proposal; accepted credit reads use the recipient acquisition's due time, at least 60 seconds after the preceding acquisition completes | Preparation, startup, requested refresh and accepted credit checks share the Store/wallet acquisition and cooldown. Reusing its result does not republish a scalar estimate. Fresh allocations and unsigned quotes do not create Base polling; SDK capital shortages and invalid proposals issue no destination read. Accepted preparation, signed/submitted and historically submitted source work remains observable across restart. The shared recipient acquisition owns exponential error backoff up to 300 seconds; credit scheduling adds no second backoff or jitter, and cached failures do not count as new recurring observations. Source and provider-status progress cannot shorten that schedule or clear its failures. See [wallet observation policy](wallet-rotation.md#wallet-observation-policy); scheduling grants no transaction authority or swap deadline. |

The hint lifetime and catalog default use the same
`API_CACHE_TTL_SECONDS` constant in `src/limits.rs`. A per-source catalog override
does not change the process-wide hint lifetime. Disk pricing-estimate freshness
still follows HTTP cache rules; it is separate from runtime payment price hints.

Recognized insufficient-funds headers on a signed HTTP 402 allow the existing
next-wallet traversal immediately. The unused error body is dropped: its length,
truncation or inactivity cannot veto fallback. Every authorization remains
journaled, and the next wallet obtains and validates its own challenge.

Header-wait timers also include request upload, so a slow but progressing upload
can exhaust the read allowance. For gRPC, waiting for stream capacity is separately
bounded by that setting. See the [transport limitations](network-egress.md#timeout-budgets-and-concurrency)
for details. A timeout never proves that a signed action did not execute and never
grants automatic replay authority.

## Rejection and truncation

A limit that rejects, truncates or discards input/output must produce a clear
log and explicit evidence for the affected consumer: a tool error or truncation
marker, HTTP error or CLI error. Identify the resource, limit and adjustment
setting where applicable. Do not log private content, credentials or sensitive
URLs to explain it. Tests verify consumer-visible evidence and logs.

Never present an incomplete document as complete. Display truncation is separate
from a download/memory bound. An oversized paid response may already have settled
and grants no automatic retry. These visibility requirements do not justify adding
another bound.
