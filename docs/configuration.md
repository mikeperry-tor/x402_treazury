# Configuration reference

For an operator-oriented introduction, start with the
[configuration guide](usage/configuration.md) and the
[managed-wallet quickstart](../README.md#quickstart-z-cash-wallet-with-dynamically-managed-tools).
[Advanced deployments](../README_ADVANCED.md) cover multi-listener Zcash and
Tor walkthroughs. This reference specifies TOML composition, tool selection,
wallet bindings, direct swap control, transport, caching, relay fallback, and
pricing.

Run commands from the repository root. The CLI requires an explicit subcommand;
running `x402_treazury` without arguments prints help and never starts serving.

| Command | Purpose |
| --- | --- |
| `serve --config FILE` | Start a multi-listener deployment and its background funding workers |
| `serve --provider FILE` | Serve one standalone provider (stdio by default) |
| `catalog tools --config FILE` or `--provider FILE` | Print selected tool inventories as JSON with advertised prices |
| `catalog warm --config FILE` | Download and cache catalogs; relay fallback may pay, `--direct` stays unsigned |
| `catalog tags --config FILE` or `--provider FILE` | Print available OpenAPI tags |
| `catalog route TOOL --provider FILE --args '<...>'` | Preview an HTTP request without sending it (standalone providers only) |
| `config show --config FILE` or `--provider FILE` | Resolve file settings and wallet bindings offline |
| `config check --config FILE` or `--provider FILE` | Fetch catalogs and validate selected tools without signing, probing prices, or funding |
| `config check --config FILE --explain-routing` | Print JSON validation results and per-tool source distribution decisions |
| `wallet bootstrap --config FILE` | Fund enabled managed reserves without listeners (requires `auto_fund = true`) |
| `wallet sync --config FILE` | Scan ZEC and managed Base USDC balances once, including active funding recipients |
| `wallet preview --config FILE` | Compare deployment wallet assignments with stored public metadata without changing state |
| `wallet rename OLD NEW --config FILE` | Rename a stored managed group offline while preserving its UUID and work |
| `wallet ...` | Administer the treasury |
| `sources inspect --config FILE` | Inspect persisted agent-added source records |
| `build-info` | Print build identity and provenance |

Run `x402_treazury COMMAND --help` for subcommand options. Only `serve` accepts
listener, authentication, and background-funding flags.

## Configuration file roles

`X402_TREAZURY_CONFIG` sets the default path for `--config` across `serve`,
`wallet`, `catalog`, `config`, and `sources` commands, and `--config FILE`
overrides it. Export `X402_TREAZURY_CONFIG` in the process environment because
`--env-file` loads after CLI parsing. Unset it before passing standalone
provider flags or wallet `--state-dir`, `--key-file`, and `--treasury-id`.

- **`--config FILE`:** Selects a deployment file defining sources, listeners,
  wallets, and network policy. Wallet commands accept `--config` without
  loading API sources. `wallet sync` requires stopping `serve` first, resolves
  treasury and Base settings without catalogs or automatic funding, and reports
  ZEC and Base results independently. `wallet status` reads local state offline.
  See [wallet CLI configuration](wallet-cli.md#one-configuration-for-wallet-commands).
- **`--provider FILE`:** Selects a reusable provider TOML for standalone
  serving or inspection. Cannot be combined with `--config`.
- **`--network-config FILE`:** Selects a file containing `[network]` and
  optionally `[relay_fallback]`. It replaces the entire `[network]` table for
  that command (omitted fields fall back to network defaults rather than
  inheriting from the deployment). If `[relay_fallback]` is present, it
  replaces the deployment's relay table; if omitted, the deployment's relay
  table stays intact. Relative paths inside the file resolve from that file.
  Standalone provider and wallet commands accept `--network-config` without
  loading relay providers.

Example deployments live in `examples/deployments/`, network policies in
`examples/network/`, and reusable providers in `providers/`. Relative paths
resolve from the file that declares them.

Managed serving binds listeners before starting background funding, so
available tools stay visible while wallets prepare. Set
`funding.auto_fund = false` to pause new funding; inspection commands never
spend funds.

## Bootstrap wallet policy

The optional `[wallet_bootstrap]` table lets participating managed groups share
one bootstrap group until each deferred group receives its first USDC credit:

```toml
[wallet_bootstrap]
bootstrap_group = "general"
deferred_groups = ["personal", "health", "financial"]
```

- **Validation:** Group names resolve against all configured and generated
  managed groups. Unknown groups, static groups, listing `bootstrap_group` in
  `deferred_groups`, duplicate entries, and empty `deferred_groups` fail
  validation. Omit the table to fund all groups independently from startup.
- **Inspection:** `config show` and `wallet preview` report the bootstrap
  table, `bootstrap_setup` nominal bounds (the bootstrap group alone), and the
  full multi-group initial inventory bounds offline without activating demand
  or allocating wallets.
- **First-use activation:** The first validated API call targeting a deferred
  group (including a free call, cache hit, or cap refusal) records durable
  UUID-bound demand and wakes the funding owner to arm an independent 30-120
  second funding opportunity (uniform integer seconds). Invalid or denied
  calls, discovery, catalogs, pricing, registration, help, and warming never
  activate deferred funding.
- **Permanent handoff:** Calls use the bootstrap group's wallet, cap, and
  transport until the deferred group records any positive initial USDC credit
  (including partial credit or credit on a now-retired wallet). Once credited,
  that group uses its own wallet permanently across restarts; depletion,
  refunds, or reorg repair never revert it to bootstrap. Calls never wait for
  funding or retry across groups.
- **Metadata relays and `wallet bootstrap`:** While `[wallet_bootstrap]` is
  configured, paid metadata and directory relay fees permanently use
  `bootstrap_group` (overriding `relay_fallback.wallet`), and API or raw relays
  use the API call's assigned group. `wallet bootstrap` funds only
  `bootstrap_group`'s enabled reserves without activating deferred groups or
  starting listeners; non-participating groups fund normally during `serve`.
  Without `[wallet_bootstrap]`, `wallet bootstrap` funds every declared managed
  group.

## Deployment composition and multiple listeners

A version-1 deployment file composes reusable providers into named sources and
exposes them on one or more authenticated MCP listeners.

### Catalog loading and listener startup

Deployment catalog downloads share a Tor concurrency gate controlling how many
active Tor SOCKS isolation scopes may fetch concurrently (default `2`, range
`1` to `64`):

```toml
[startup]
catalog_concurrency = 2
```

Tor maintains
[three prebuilt multipath Conflux sets](https://lists.torproject.org/mailman3/hyperkitty/list/tor-commits@lists.torproject.org/thread/NGD4PUZLR2YNGSVBS3XIGXT56OM7RAGB/)
by default, so a default of 2 leaves headroom for replenishment (Conflux
prebuilding is consensus-controlled under
[Proposal 329](https://spec.torproject.org/proposals/329-traffic-splitting.html)).
See [startup tests](../tests/STARTUP.md).

- **Slot accounting:** Catalog, pricing, and relay requests sharing the same
  SOCKS isolation credentials share one slot and run in parallel. Distinct
  transport or retry scopes count separately even on the same domain. A slot is
  held until its last active response body finishes downloading, and released
  before retry delays or wallet waits. Cache hits, local files, relay readiness
  waits, and direct traffic (including `catalog warm --direct`) consume no
  slots.
- **Listener binding and `wait_for_catalogs`:** Serving loads local catalogs
  and binds every listener socket before starting background funding and remote
  catalog fetches. Each listener defaults to `wait_for_catalogs = true`,
  accepting MCP HTTP requests only after its bound sources settle (other
  endpoints do not block it). Set `wait_for_catalogs = false` to serve
  available tools immediately and publish remote catalogs as they arrive:

```toml
[servers.research]
listen = "127.0.0.1:8000"
sources = ["company_stableenrich"]
wallet = "research"
bearer_token_env = "X402_TREAZURY_MCP_TOKEN"
wait_for_catalogs = true # Default; false publishes remote tools incrementally.
```

Because sockets bind early, TCP connections can queue while MCP HTTP waits.
`MCP listening at /mcp` logs only when an endpoint begins serving MCP requests.
Remote spec aliases within one deployment load share a single download and
parsed document when their exact URL, timeout, byte limit, transport flags, and
discovery identity match, while keeping separate tools, filters, base URLs, and
wallet bindings. Local files always read independently.

#### Startup announcement

Once every endpoint is listening after successful catalog startup and every
bound managed pool has at least one eligible payer whose saved USDC estimate
covers its finite `max_api_payment_usdc`, serving logs one `INFO` line:
`All MCP endpoints listening, funded, and ready` (tagged with `funding_basis`).

- Unused pools and full reserve inventory are not required; balance and credit
  notifications can trigger the announcement after swaps settle.
- Static payers (no saved balance observations), unlimited payment caps, and
  required catalog or selection failures suppress the announcement.
- The announcement is advisory and runs no extra network reads or polling.
  There is no overall catalog-startup deadline: progress, `Retry-After` waits,
  Tor startup, or relay readiness can keep a waiting endpoint waiting while
  shutdown still cancels unsigned fetches and drains accepted paid work.

#### Catalog failures and strict inspection

- **Required remote failures:** If a remote source fails to load and does not
  set `allow_catalog_failure = true`, waiting endpoints that depend on it close
  their sockets and emit `ERROR` naming each failed endpoint and source.
  Unrelated endpoints keep running. Restart to retry failed sources.
- **Tolerant remote failures:** Setting `sources.NAME.allow_catalog_failure = true`
  (or `wait_for_catalogs = false` on the listener) omits a failed remote source
  as a unit and lets the endpoint serve healthy siblings while reporting
  pending or unavailable sources in stderr and MCP instructions.
- **Local fatal errors:** Malformed configuration, unreadable local files,
  malformed local documents, and structural-safety violations fail startup.
- **Permissive selection in serving:** Serving accepts empty source or listener
  selections (missing tool names, missing tags, contradictory filters, empty
  opted-in `recommended_tools`, or all-unsupported operations), emits `ERROR`
  coverage diagnostics after the listener's sources settle, and lists gaps in
  MCP instructions without widening filters. Unmatched exclusions log at
  `INFO`.
- **Ambiguous tool names:** If two sources on the same listener publish the
  same tool name after tag selection, serving omits every binding of that name
  and reports the collision in MCP instructions. Calls already in flight finish
  with their active binding.
- **Reserved tool names:** A provider tool in the `x402_treazury_` namespace
  makes its source unavailable before routing. The `dyn_` namespace is reserved
  across the deployment when any listener enables provider management or raw URL
  calls, and on any listener that enables on-demand discovery or configures swap
  control. During serving, any source whose selected tools violate a reserved
  namespace is marked unavailable as a unit (leaving unrelated listeners and
  healthy siblings unaffected), and each dependent listener applies its catalog
  readiness settings.
- **Strict inspection:** `config check`, `catalog tools`, deployment
  `catalog tags`, and qualification require complete catalogs and reject
  missing exact names, empty inventories, duplicate names, and reserved
  namespaces. `config show` runs offline and prints effective
  `catalog_concurrency` and `wait_for_catalogs`.
  Progress logs go to stderr (with a 10-second waiting message for remaining
  work) without response bodies or URLs. See
  [framework integration](framework-integration.md).

### Example deployments and serving

- [examples/deployments/managed.toml](../examples/deployments/managed.toml):
  one authenticated `research` listener, Exa search and contents, one managed
  pool, direct networking, and ephemeral agent-added APIs. See the
  [quickstart](../README.md#quickstart-z-cash-wallet-with-dynamically-managed-tools).
- [examples/deployments/comprehensive.toml](../examples/deployments/comprehensive.toml):
  four-listener Tor deployment with curated sources, bootstrap funding, relay
  fallback, and persistent agent-added APIs (`source_registry_file`).
- [examples/deployments/research_desk.toml](../examples/deployments/research_desk.toml):
  twelve-listener Tor deployment with curated sources, bootstrap funding, relay
  fallback, and ephemeral agent-added APIs. See
  [README_ADVANCED.md](../README_ADVANCED.md) and
  [examples/README.md](../examples/README.md).

### Listener transport

Each deployment listener serves stateless Streamable HTTP at `/mcp` on its
configured bind address (use a literal such as `0.0.0.0` in containers).
There is no hot reload; restart to apply config changes.

Static wallet profiles use `mode = "static"` with `private_key_env` and require
no `[treasury]`. Static serving loads only the private keys used by selected
tools.

### Deployment file structure

Version-1 deployment files define top-level `sources`, `wallets`, and
`servers`, plus optional `wallet_templates` and `wallet_assignment`:

- **`sources`:** Provider settings written inline or imported via
  `extends = "../providers/pdl.toml"`. Supports all generic catalog settings
  (`spec`, `name`, `description`, `base_url`, `prefix`, `read_timeout_seconds`,
  path and tag filters, `include_tools`, `exclude_tools`, pricing, overrides,
  and guidance) plus deployment-only `wallet` and `send_tools` (both forbidden
  in reusable provider files).
- **`wallets`:** Either `mode = "static"` (`private_key_env`,
  `max_api_payment_usdc` defaulting to `"1.00"`) or `mode = "zcash_rotation"`
  (requires explicit `max_api_payment_usdc`). Bindings referencing the same
  wallet share payer state; `max_api_payment_usdc` caps a single payment, not
  cumulative spend.
- **`servers`:** `listen`, `sources`, `bearer_token_env` (required when
  `auth = true`, the default), optional default `wallet`, and optional
  `include_tools`, `exclude_tools`, `tags`, and `exclude_tags`. Identifiers use
  lowercase letters, digits, and `_`, starting with a letter.

### Source selection and tool filters

- **`description`:** Optional searchable metadata for `config show` and bundled
  provider listing (required nonblank on bundled providers; see
  [bundled provider metadata](../providers/README.md#bundled-provider-metadata)).
  Inherits and replaces as a whole field; does not replace tool descriptions or
  `instructions_text`.
- **`include_operations` and `exclude_operations`:** Match exact original
  `METHOD path` entries (uppercase `GET`, `POST`, `PUT`, `PATCH`, or `DELETE`)
  independently of generated names and prefixes. Exclusions take precedence
  before method preference and naming. Unlike subtree `exclude`, excluding a
  parent path keeps supported child routes. Lists replace on inheritance (`[]`
  clears).
- **`prefer_post`:** Defaults to `true`. After path, operation, and tag
  compatibility filters, exact-path `GET` and `POST` twins keep `POST` before
  collision counting and naming (an unsupported `POST` never hides a compatible
  `GET`). Suppressed `GET` twins do not appear in omission diagnostics. Set
  `prefer_post = false` to expose both methods with `_get` and `_post`
  suffixes.
- **`include_tools` and `exclude_tools`:** Match generated tool names using
  case-sensitive `*` and `?` patterns after path, operation, and tag filtering,
  so excluding a tool never renames its siblings. Exclusions take precedence.
  Help tools participate in name filters. Strict inspection rejects unknown
  exact inclusions and empty selections (unmatched wildcards warn); serving and
  warming report missing selections as reduced coverage and log unmatched
  exclusions at `INFO`. Listener filters narrow source selections further and
  cannot restore excluded tools:

```toml
[sources.company]
extends = "../../providers/otto.toml"
include_tools = ["otto_equity_intel", "otto_financial_statements", "otto_help"]
```

- **`recommended_tools` and `use_recommended_tools`:** `recommended_tools`
  holds an advisory list using the same generated-name syntax. All compatible
  tools stay visible unless `use_recommended_tools = true` (default `false`):

```toml
[sources.research]
extends = "../../providers/otto.toml"
recommended_tools = ["otto_equity_intel", "otto_financial_statements", "otto_help"]
use_recommended_tools = true
```

  Opt-in intersects recommendations with source filters (exclusions win).
  Lists replace on inheritance (`recommended_tools = []` clears); changing a
  source's `prefix` does not rewrite inherited names. `catalog tools` marks
  each tool with `recommended: true` or `false` without changing MCP schemas,
  and `catalog tools --all-tools` bypasses only `use_recommended_tools` while
  keeping path, operation, tag, name, and listener filters intact. Unused stale
  recommendations warn without hiding compatible tools; malformed selector
  syntax always fails validation.

### Listener settings, host validation, and authentication

- **Host validation:** Defaults to `localhost`, `127.0.0.1`, and `::1`
  regardless of bind address. Override per listener with `allowed_hosts` (exact
  hostnames or IPs, optionally with `:port`; no URLs or wildcards; replaces the
  default list) or `disable_host_check = true` (cannot be combined with
  `allowed_hosts`; `allowed_hosts = []` also disables Host checking).
  Standalone HTTP accepts `--allowed-hosts` and `--disable-host-check` with
  `--transport http`. Disabling Host checking emits a startup warning.

```toml
[servers.research]
allowed_hosts = ["mcp.example.com", "localhost"]
# Or disable Host validation (requires auth = true):
# disable_host_check = true
```

- **Authentication:** Enabled by default (`auth = true`). Every authenticated
  listener requires a nonempty value in `bearer_token_env` (such as
  `X402_TREAZURY_MCP_TOKEN`) at startup. To disable authentication on one
  listener, set `auth = false` and omit `bearer_token_env` (standalone HTTP
  uses `--transport http --no-auth`). Disabling both authentication and Host
  checking (including `allowed_hosts = []`) is rejected. For standalone HTTP,
  prefer `X402_MCP_BEARER_TOKEN` over `--bearer-token` so the token does not
  appear in process arguments.
- **Bearer header handling:** Clients send `Authorization: Bearer <token>`
  (case-insensitive scheme, exact constant-time token comparison). HTTP `401`
  distinguishes missing, malformed or duplicate, and incorrect headers.
  Warnings log without credentials at most once per 30 seconds per listener and
  failure category (reporting suppressed counts on the next warning).
- **On-demand discovery and instructions:** Set `discover_on_demand = true`
  (default `false`) to advertise `x402_treazury_tools_search` and
  `x402_treazury_tool_call` in place of individual provider API schemas.
  `tools/list` includes enabled built-in tools, including provider management,
  raw calls, and direct swap control. Enabled swap tools also appear in tool
  search and can be invoked through the call wrapper.
  Search uses BM25 keyword ranking with literal-substring priority (25 results
  per page by default); see
  [tool search matching](agent-sources.md#tool-search-matching-and-results).
  `instructions_text` overrides composed source guidance (an empty string
  suppresses it) while preserving pricing and operational notices.
  `discovery_search_tool_instructions_text` and
  `providers_search_remote_tool_instructions_text` append guidance to those
  tool descriptions without enabling discovery or management; see
  [endpoint instruction placement](agent-sources.md#endpoint-instruction-placement).

### Wallet bindings

For each listener-source binding, wallet precedence is **source `wallet` ->
server default `wallet` -> automatic assignment**. A server may omit its
default `wallet` if every bound source sets `wallet` or `[wallet_assignment]`
provides a fallback. Missing assignments and unknown wallet names fail
validation even when tools are filtered out.

| Arrangement | Wallet sharing |
| --- | --- |
| All bindings reference one wallet | Shared across the deployment |
| Each server has its own default, with no source overrides | Shared within each server |
| Each source specifies its own wallet | Shared across listeners using that source |
| Several sources reference one wallet | Shared within an operator-defined group |

```toml
[sources.socialfetch]
extends = "../providers/socialfetch.toml"
wallet = "social"

[sources.pdl]
extends = "../providers/pdl.toml"

[servers.research]
listen = "127.0.0.1:8000"
bearer_token_env = "RESEARCH_MCP_TOKEN"
wallet = "general"
sources = ["socialfetch", "pdl"]
```

To isolate two copies of the same provider into separate wallets, define two
source aliases extending that provider with different `wallet` names.
Referencing one profile from multiple bindings never creates extra pools or
funding jobs.

- `config show`, `catalog tools`, and `config check` report `wallet_bindings`
  with origins (such as `sources.socialfetch.wallet`) offline without reading
  secrets or treasury state.
- `wallet preview --config FILE` compares validated assignments against stored
  public metadata without modifying state or unlocking keys; see
  [wallet preview](wallet-cli.md#preview-configuration-against-saved-groups).
- Managed wallet names map to stored group UUIDs. Renaming a wallet in TOML
  creates a new empty group; use `wallet rename OLD NEW --config FILE` with
  `serve` stopped to keep the same identity and funds, then update TOML
  references. See
  [wallet rename](wallet-cli.md#rename-a-managed-wallet-group).

### Wallet templates and automatic assignment

Define `[wallet_templates.NAME]` and `[wallet_assignment]` to generate managed
wallet groups automatically without repeating `[wallets]` blocks:

```toml
[wallet_templates.small]
mode = "zcash_rotation"
max_api_payment_usdc = "1.00" # Required per-payment cap.
max_retirement_remainder_usdc = "0.001" # Default per group; "0" requires exact zero.

[wallet_templates.small.reserves.small]
# Default randomized nominal funding: $2-3.

# Optional large reserve (default $10-25 when declared):
# [wallet_templates.small.reserves.large]

[wallet_assignment]
scope = "source"
template = "small"
```

| Automatic scope | One generated pool per | Sharing |
| --- | --- | --- |
| `deployment` | Deployment treasury | All otherwise-unassigned bindings |
| `server` | Listener name | Otherwise-unassigned sources on that listener |
| `source` | Source name | That source across otherwise-unassigned listeners |
| `binding` | Listener and source pair | Each otherwise-unassigned binding separately |

- `[treasury]` and `[funding]` are still required when resolved pools are
  managed. Explicit source and server `wallet` references take precedence and
  must reference `[wallets]` (not templates); provider management and raw calls
  still require an explicit listener `wallet`.
- Templates validate the same fields as `[wallets]` (static templates are
  rejected) and allocate nothing unless referenced by an unassigned binding.
  Tool filters never suppress or alter generated pool names. Explicitly
  declared `[wallets]` profiles always request pools even if unreferenced.
- Generated names use the reserved `auto_v1_` prefix (`auto_v1_deployment`,
  `auto_v1_server_<server>`, `auto_v1_source_<source>`, and
  `auto_v1_binding_<len>_<server>_<source>`), which explicit `[wallets]`
  entries cannot use. Pool identity belongs to the initialized treasury state
  directory, not the config filename or template name.
- Managed profiles require an explicit `max_api_payment_usdc` (use `"none"` or
  `"off"` for unlimited; empty strings fail). The cap applies to active,
  reserve, and retained wallets alike and is independent of reserve sizing.

### Managed reserves and funding

- **Reserve slots:** `reserves.small` defaults to a randomized $2-3 nominal
  range when omitted or empty. `reserves.large` is disabled when omitted and
  defaults to a randomized $10-25 range when declared empty. Each slot accepts
  either a fixed `funding_amount_usdc` (with optional `max_funding_amount_usdc`
  headroom for bridge minimums) or both `min_funding_amount_usdc` and
  `max_funding_amount_usdc` (inclusive exact micro-USDC range). Do not combine
  `funding_amount_usdc` with `min_funding_amount_usdc`.
- **Lifecycle:** New groups fund only their enabled reserve slots (small first
  when both are vacant). The first journaled payment promotes a `READY` wallet
  to `ACTIVE` and opens its slot for refill; displaced `ACTIVE` wallets become
  `RETAINED` and remain spendable without top-ups. A wallet's first positive
  USDC credit (even partial) permanently consumes its initial funding claim.
- **Quote sizing and bridge floors:** Each funding job draws its nominal USDC
  amount (fixed target or uniform draw in range) and previews the ZEC principal
  via a dry `EXACT_OUTPUT` quote with zero slippage. If NEAR reports a higher
  bridge minimum within the configured maximum, the job raises its local floor,
  redraws within the remaining authorized range (or adjusts a fixed target),
  and persists the choice before retrying. If the bridge minimum exceeds the
  configured maximum, the job stops that round with an amount-limit error and
  retries on normal backoff without silently raising the ceiling. The
  executable `EXACT_INPUT` quote locks that ZEC principal and applies
  `funding.slippage_bps` (default `500`, range `0` to `10000`) once to its
  expected output.

#### Retired-wallet tolerance

`max_retirement_remainder_usdc` defaults to `"0.001"` (1,000 atomic units) per
managed group; set `"0"` to require an exact zero balance.

- During serving, low-estimate `RETAINED` wallets whose finalized and latest
  balances both fit the tolerance and have no unresolved payment authorizations
  or pending delivery obligations retire automatically (regardless of
  `auto_fund`). Active, ready, and disabled-group wallets never retire.
- A never-credited job closes its delivery guard only when every bound swap
  reached a terminal NEAR outcome with no pending source work, or nondisclosure
  is proven.
- `wallet sync` checks eligible candidates once without starting a watcher.
  Retirement leaves keys and any dust at their address, stops routine polling
  and selection, and never transfers funds.

#### Base finality and schema

Canonical Base balances, authorization resolution, and payment receipts read
the RPC's `finalized` block tag (there is no separate confirmation-depth
setting). If `finalized` lags or is unavailable, pending work waits and may
fail over across configured Base RPC endpoints rather than substituting
depth-only checks. Advisory balance estimates still read `latest`.

Current SQLite schema version 23 is required for both owner opens and read-only
inspection; older and newer schemas are rejected without mutating state. See
[database schema compatibility](wallet-cli.md#database-schema-and-upgrade-compatibility).

#### Funding capital and automatic funding

Spendable shielded ZEC in the treasury (after confirmations, reservations,
liabilities, and SDK fees) is the ZEC capital limit for enabled auto-funding.
Reserve ranges size nominal USDC targets only.

- **`[treasury]` keys:** `state_dir` (required), `key_file` (optional,
  default `state_dir/wallet.key`), `id` (optional expected UUID verified on
  open), `indexer_url` or `indexer_url_env` (mutually exclusive; default
  `https://zec.rocks:443`), `submission_url` or `submission_url_env` (mutually
  exclusive; defaults to the indexer endpoint), and `confirmations` (default
  `3`, positive integer). Missing or empty referenced environment variables
  fail validation.
- **`[funding]` keys:** `auto_fund` (default `true`), `confidentiality`
  (default `"basic"`, `"public"` opt-in, `"advanced"`), `base_rpc_url_env` and
  `base_rpc_fallback_url_envs`, `near_api_key_env`, `near_user_session_env`,
  `slippage_bps` (default `500`), `poll_seconds` (default `5`),
  `swap_timeout_seconds` (default `1800`), and `quote_deadline_seconds`
  (default `1800`).
- Unknown and obsolete fields (such as `max_funding_spend_zec`,
  `max_response_chars`, `treasury.max_sync_age_seconds`, and
  `funding.base_max_block_age_seconds`) fail TOML parsing and must be removed
  from configurations.
- `funding.auto_fund = false` pauses new funding while continuing to observe
  already accepted operations. Unfilled reserve claims keep watching for new
  ZEC deposits; fully funded idle serving does not poll for incoming ZEC.
- Live qualification has no separate ZEC budget; its `total_test_budget_usdc`
  caps API authorizations independently.

### Reserve settings and state changes

Config edits take effect on restart and govern future allocations without
converting wallet state or modifying already accepted swaps or signed bytes.
Changing assignment scope or renaming a source or server in TOML creates new
groups and disables absent groups while keeping their keys and history intact.

`config show` and `config check` include `resolved_wallets`,
`generated_wallets`, and `wallet_summary` with combined nominal lower and upper
bounds across distinct managed pools (for example, $12-$28 for one group with
default small and large reserves). The `initial_inventory_target_*` fields hold
the nominal lower bound for compatibility. None of these inspection commands
opens keys or allocates funds.

Refunded managed swaps keep polling NEAR and scanning Zcash until confirmed
Ironwood receipt arrives at the saved refund receiver. Only a bound `REFUNDED`
status plus source closure and confirmed receipt allow a replacement funding
swap under an unused initial claim. See
[refunds, state, and recovery](wallet-rotation.md#refunds-state-and-recovery).

### Funding timing and observation cadence

- **`funding.poll_seconds`:** Default `5` (range `1` to `60`). Controls local
  funding scheduling and successful recurring source and NEAR status checks
  (with per-job jitter). Failed observations back off from a 60-second floor up
  to 300 seconds before jitter. Base credit checks keep a 60-second completion
  floor.
- **Treasury scans:** Requested at startup, explicit `wallet sync`, enabled
  funding demand, and pending source, refund, or direct receive work.
  Successful scans schedule the next recurring scan 60 seconds after completion;
  failures back off at 90, 180, and 300 seconds. Idle treasuries do not poll.
  See [wallet observation policy](wallet-rotation.md#wallet-observation-policy).
- **`funding.swap_timeout_seconds`:** Default `1800` (from first durable
  broadcast intent). Marks pool health degraded without abandoning accepted
  work or authorizing a replacement swap.
- **`funding.quote_deadline_seconds`:** Default `1800`. Requests a quote window
  for preparation and disclosure independently of the Zcash SDK's block-height
  transaction expiry. Accepted deadlines and signed bytes remain immutable
  across restarts.

### Reserve sizing and fragmentation

Size `reserves.large` to at least 2x your largest expected payment, and
preferably 4x when prices vary. Keeping a separate large reserve lets cheap
calls continue rotating on small wallets while expensive calls draw from the
large reserve.

- **Fragmentation:** Balances across separate wallets cannot be combined to pay
  one challenge. For target `L` and repeated settled payments `p`, the leftover
  remainder unable to cover another payment `p` is `L - floor(L / p) * p`.
  Keeping payments at or below 25% of `L` keeps remainders under 20% for fixed
  prices and under 25% for mixed prices (early rotation or pending
  authorizations can leave larger remainders, which later smaller calls can
  still use).
- **Small-reserve capital planning:** A default $2-3 small allocation averages
  $2.50 nominal. Funding a group's first wallet plus its first replacement
  spans two allocations ($4-6 nominal, about $5 on average before bridge
  minimums and fees). Allow headroom up to $6 plus fees, ongoing refills, and
  any large reserves. If a bridge minimum exceeds $3, `x402_treazury` refuses
  the route rather than silently raising the $3 ceiling.
- **Payer selection summary:** Before the first unsigned request, managed
  selection checks a fresh observed offer, then a valid advertised per-call
  price. With a hint, it prefers estimated-affordable `RETAINED` wallets
  (oldest first), then `ACTIVE`, then `READY`. Without a hint, it prefers
  healthy positive `ACTIVE`, then `RETAINED`, then `READY`, with low, zero, and
  unknown estimates as fallbacks.

### Startup readiness and configuration changes

Serving binds listener sockets before starting background funding, and MCP
availability follows each listener's `wait_for_catalogs` setting independently
of wallet readiness.

- **Refill opportunities:** Eligible groups wait for an independent 30-120
  second refill opportunity (uniform integer seconds, mean 75) rather than
  allocating immediately on startup or exhaustion. Already allocated work
  resumes promptly on restart. At an opportunity, a used `ACTIVE` wallet that
  dropped to its private 5%-25% initial-credit threshold (scaled by the current
  cap) can promote a funded `READY` reserve; see
  [early rotation](wallet-rotation.md#early-rotation-and-refill-opportunities).
- **Serving with `READY` wallets:** A funded `READY` wallet serves immediately
  without needing a preexisting `ACTIVE` wallet; unsigned requests never
  promote wallets. An empty managed pool returns a typed unavailable error
  until a funded payer exists.
- **Reopen and schema discipline:** Restarting applies updated caps, reserve
  bounds, and slot enablement to future allocations. Unprepared quotes that no
  longer fit updated policy are refreshed; prepared transactions retain their
  exact inputs, amounts, and signed bytes.

Configuration is TOML-only; reusable definitions live in
[providers/](../providers/README.md) (with locally curated `provider.toml` and
`openapi.json` pairs for providers such as AgentUtility, Glassnode,
Concordance, Straits, and Locus). Small PNG, JPEG, and WebP responses become
MCP image attachments, and provider `response_mappings` can extract inline
base64 or data-URI images from JSON; see
[inline image results](../providers/README.md#inline-image-results).

### TOML composition and filtering

Composition is single-level: a deployment source may `extends` a provider file,
and provider files cannot themselves use `extends`.

- **Field inheritance:** Local fields replace inherited fields completely
  (arrays do not concatenate; `overrides` and `response_mappings` replace in
  full). Empty lists or tables clear inherited values. Omitted fields inherit,
  then fall back to defaults. Unknown fields fail parsing.
- **Path resolution:** `extends` resolves relative to the deployment file; a
  provider's local `spec` resolves relative to the provider file; a
  deployment's local `spec` override resolves relative to the deployment file.
- **Prefixes and filter order:** A provider's explicit `prefix` stays intact;
  otherwise the source ID becomes the prefix. Source path, operation, and tag
  filters run first, then `send_tools` (when present), then listener tag and
  name filters. Tag matching is exact and case-sensitive (exclusions win;
  untagged tools including help do not match positive tag filters).
- **Filter diagnostics:** After a listener's sources settle, diagnostics
  identify unmatched `include_tools`, `exclude_tools`, source filters, or
  winning `send_tools[index]` rules that removed matching tools. Sources shared
  across listeners load once, and their instructions combine without silent
  truncation.

#### Edit inherited tool metadata

Use `overrides_patch` to change individual fields while inheriting the other
tool edits. Use `response_mappings_patch` to replace the complete mapping for a
method and path.

```toml
[sources.example]
extends = "provider.toml"

[sources.example.overrides_patch.example_search]
pricing_description = false

[sources.example.overrides_patch.example_search.params]
query = "Terms to search for."

[sources.example.response_mappings_patch."GET /search"]
omit = ["/debug"]
```

Set a tool, field, parameter note, or operation mapping to `false` in the
corresponding patch table to remove that inherited value. A removed description
or parameter note uses the catalog text. Response mapping arrays preserve their
declared order. An empty patch leaves the inherited map in place. To clear the
whole map, use `overrides = {}` or `response_mappings = {}`. Declare either the
whole map or its patch in each source.

`config show` reports mixed map origins with JSON Pointer keys relative to
the effective settings. A tool edit origin such as
`/overrides/example_search/params/query` identifies its declaring file. A
response mapping origin names the complete operation mapping.

Use patch tables in a deployment source or a standalone configuration file.
Provider files loaded through `extends` declare `overrides` and
`response_mappings` directly.

For standalone serving, `X402_MCP_GENERIC_PRICING_KEY` replaces the file's
pricing declaration with the legacy catalog key. Standalone `config show`
reports file settings.

### Source tool distribution

Deployment sources support optional ordered `send_tools` rules to route tools
to specific endpoints or exclude them. Routed and listener-`sources` sources
may coexist in a deployment, though a source using `send_tools` cannot also
appear in any listener's `sources` list. Reusable provider files, dynamic
imports, and `raw_call` do not accept `send_tools`.

```toml
[sources.vaaya]
extends = "../../providers/vaaya.toml"
send_tools = [
  { to = "exclude", tools = ["supersearch", "consult", "root"] },
  { to = ["company", "research"], tools = ["edgar_*"] },
  { to = "web" },
]
```

- **Rule structure:** `send_tools` must be nonempty. Each rule takes required
  `to` (one endpoint name, a nonempty array of distinct endpoint names, or
  scalar `"exclude"`) and optional nonempty `tools`. Exactly one rule must omit
  `tools` and must appear last as the catch-all. Using `send_tools` reserves
  the endpoint name `exclude`.
- **Pattern matching:** Selectors use case-sensitive `*` and `?` against
  generated names with the resolved `<prefix>_` stripped once (for prefix
  `locus`, `locus_locus_zoning` matches `locus_zoning`, `locus_list` matches
  `list`, and `locus_help` matches `help`). Callable names are unchanged.
- **Evaluation order:** Source compatibility, path, operation, and tag filters,
  and recommendation opt-in run first. The first matching rule consumes each
  surviving tool (sending it to every listed endpoint or excluding it). Later
  rules cannot reclaim a consumed tool. Every non-exclude target in any rule
  establishes a potential binding for wallet resolution and startup
  dependencies; an exclusion-only source adds no endpoint bindings.
- **Validation and diagnostics:** Strict inspection rejects missing exact
  assignment selectors and empty inventories (unmatched wildcards log at
  `DEBUG`). Serving and warming accept missing selectors and empty inventories
  with `ERROR` coverage diagnostics (unmatched `exclude` selectors log at
  `INFO`, and fully shadowed rules log at `DEBUG`).
- **Inspection:** `config show` prints configured rules and effective bindings
  with origins such as `sources.vaaya.send_tools[1].to`.
  `config check --explain-routing` prints one JSON object (`wallet_summary`,
  `servers`, and `routing`) detailing each surviving tool's winning rule,
  routed targets, post-filter targets, and wallet bindings without reading
  secrets or allocating wallets.

### Graceful shutdown

Startup validates credentials and binds every listener before serving (a bind
failure releases already-bound sockets). On `SIGINT` or `SIGTERM`, serving
stops accepting new requests and drains accepted calls with 10-second stderr
progress reports and no fixed drain deadline. A second `SIGINT` or `SIGTERM`
forces exit with an unknown-outcome warning without releasing reservations or
replaying signed work.

Client disconnects and stdio EOF stop response delivery without abandoning
in-flight calls. A listener failure stops sibling listeners; a required catalog
startup failure disables only dependent waiting endpoints.

## Direct swap control

All Zcash operations (both direct swaps and managed funding) follow the
[Ironwood-only policy](zingo-integration.md#ironwood-only-policy). The sole
transparent-output exception is a NEAR-issued deposit address paid from
Ironwood under its saved quote; wallet transparent receiving, shielding, and
transparent-address RPC queries are unsupported.

Enable direct native ZEC and foreign-asset tools per authenticated listener
(`auth = true`) with an initialized `[treasury]`:

```toml
[servers.toolbox.swap_control]
receive = true
send = true
# max_send_zec = "0.05" # Optional per-send total debit; omit or use "none" for no cap.
```

- **Prerequisites and persistence:** Native shielded ZEC transfers require only
  `auth = true` and `[treasury]` (no NEAR or Base setup). Foreign swaps also
  use `[funding]` NEAR settings (credentials, slippage, and quote window).
  Omitting `swap_control` disables new sends and receives; persisted operations
  stay inspectable via `x402_treazury_swap_status` on their original listener
  even after `swap_control` is disabled or removed, and background observation
  continues even after listener removal. Keep `receive = false` and
  `send = false` in TOML if you need strict inspection to see a status-only
  inventory. If the treasury SDK is unavailable at startup, MCP advertises
  saved status only. If the SDK starts without a valid NEAR configuration,
  MCP advertises native ZEC inputs for enabled sends and receives. Inspection
  reports the configured tools.
- **Principal and isolation:** The authenticated listener owns its requests;
  all callers sharing its token share that principal. `auto_fund`, raw calls,
  and provider management never grant swap permissions.
- **`x402_treazury_swap_assets`:** Always includes native ZEC (`Zcash`,
  `nep141:zec.omft.near`, chain `zec`) locally; foreign asset metadata reports
  availability explicitly. Nonblank queries use literal-priority BM25 search
  over asset ID, chain, symbol, CoinGecko ID, contract address, name, and
  description (use spaced terms such as `base USDC`); blank queries return
  native-first provider order.
- **`x402_treazury_payment_request_create`:**
  - **Native ZEC (`nep141:zec.omft.near`, chain `zec`):** Omit
    `refund_address` and `refund_after_seconds`. Derives a fresh
    Orchard-receiver-only Unified Address (reusable; partial receipts
    accumulate until confirmed funds reach the requested amount). Retrying the
    same `request_id` with identical terms returns the original address.
  - **Foreign assets:** Requires a payer-owned `refund_address` and optional
    `refund_after_seconds` (default `7200`, two hours). Uses an `EXACT_INPUT`
    quote (Stellar sources use `MEMO` deposit mode; others default to `SIMPLE`
    while keeping any mandatory memo).
    - Tell the payer to send **exact** `amount` atomic units plus network fees
      and any mandatory memo, once only. Never reuse a foreign swap deposit
      address: each new payment requires a new `request_id` and quote.
    - Retrying an identical `request_id` returns the saved operation;
      conflicting terms fail. An uncertain quote creation never authorizes
      another quote.
- **`x402_treazury_swap_status`:** Accepts `operation_id` or `kind` and
  `request_id`. Reports native SDK receipts (using `receipt_matches_saved_snapshot`
  when refreshing from a newer snapshot without triggering a scan) or saved
  foreign quotes, provider outcomes, and separate wallet observations. Includes
  `payment_instructions`, `current_time_utc`, UTC-Z timestamps, and nonnegative
  `seconds_until_*` countdowns (`deposit_address_cold_at_utc` and
  `seconds_until_deposit_address_cold` signal slower processing, not expiry).
- **`x402_treazury_swap_preview`:** Non-executable (`executable = false`,
  reserves no inputs, creates no proof). Reports principal, `max_send_zec`, and
  `fee_estimate` (advisory spendable Ironwood balance, ZIP-317 fee, total,
  `available_debit_zatoshis = min(spendable_balance, max_send_zec)`, and
  `fits`). When balance or fee data is unknown, `fits` is `null` unless the
  SDK reports insufficient funds or the debit already exceeds a known limit.
  The minimum SDK fee provides a lower bound when the estimated fee is unknown.
  Foreign previews run a dry `EXACT_OUTPUT` quote against an existing
  shielded refund receiver with fee basis `near_deposit_output_shape`.
- **`x402_treazury_swap_send`:** Sends native ZEC to a shielded-only recipient
  or runs an internal `EXACT_OUTPUT` NEAR quote for a foreign amount (no
  destination memos or tags). `swap_control.max_send_zec` caps principal plus
  the SDK-calculated network fee (`"none"` or omitted means unlimited; `"0"`
  denies new sends) and applies only to direct swap sends, never managed
  funding. Identical `request_id` retries return the original operation;
  conflicting reuse fails. An uncertain quote creation or submission never
  authorizes another quote, replacement deposit, or signature, and a provider
  refund is a provider outcome rather than authority to pay again. Accepted
  work survives disconnects, disablement, and restarts through the shared
  treasury owner and signed-byte journal.

## Download and display limits

Provider and source tables accept these positive byte limits:

| Setting | Default | Applies to |
| --- | --- | --- |
| `max_response_bytes` | 16 MiB (`16777216`) | API response bodies, including payment challenges and errors |
| `max_help_bytes` | 4 MiB (`4194304`) | Lazy help documents and `documentation_url` reads |
| `max_spec_bytes` | 32 MiB (`33554432`) | Remote OpenAPI specifications (local files are unaffected) |

Standalone CLI overrides are `--max-response-bytes`, `--max-help-bytes`, and
`--max-spec-bytes`. Generic agent-added sources, `raw_call`, and
`documentation_url` reads use the defaults; bundled imports keep their provider
limits. Agents cannot change these limits.

- **Enforcement:** Both declared `Content-Length` and streaming reads enforce
  the cap. Oversized downloads are rejected whole with an error naming the
  limit and setting, logged to stderr, and never cached or retried for payment.
- **Structural safety budget:** All OpenAPI documents (local, remote, cached,
  or relayed) share a limit of depth 64, 500,000 nodes, and 64 MiB of string
  and key traversal work across `$ref` expansion; exceeding it rejects the
  document whole.
- **Display limits:** Completed tool output is returned in full except for
  explicit JSON field omissions (disclosed in the result). The obsolete
  `max_response_chars` setting and `--max-response-chars` flag fail parsing.
  See [tool description limits](#observed-prices-and-description-overrides)
  for `max_description_chars`. Incoming MCP HTTP requests have a 4 MiB body
  limit (HTTP `413` with a stderr warning).

### JSON response projection

Sources and providers can omit verbose JSON object fields locally by exact HTTP
method and original OpenAPI path:

```toml
[response_mappings."GET /search"]
omit = ["/results/*/figures"]
```

- Paths use RFC 6901 pointer escaping (`~0` for `~`, `~1` for `/`), with `*`
  traversing array elements (never matching object keys). Only object fields
  can be removed, not array elements. Empty root paths and invalid escapes fail
  config load.
- Each tool result advisory lists every path that removed fields and its
  removal count. When a mapping extracts images and omits fields on the same
  operation, image extraction runs first on the same parsed response.
- Projection runs locally after the response arrives (never issuing another
  request) and does not affect `raw_call`. Overriding `response_mappings`
  replaces the inherited table in full (`{}` clears). Agents cannot supply
  mappings. See [response mappings](../providers/README.md#response-mappings).

## Supported behavior and CLI defaults

- **Formats and schemas:** TOML providers, local or remote OpenAPI 3 JSON,
  local `$ref` resolution, path and tag filters, deterministic tool names,
  flattened object schemas, lazy cached `help_url` tools (unpaid fetch failures
  can retry; failures with possible relay charges stay cached), and
  `instructions_text`.
- **Payments:** Base canonical USDC. Static profiles support x402 v1 and v2
  `exact` plus v2 `upto` (Permit2 `upto` requires an external allowance and
  never auto-approves one); managed profiles support x402 v1 and v2 `exact`
  EIP-3009 only.
  - V2 challenge descriptions cap at 500 characters for facilitator
    compatibility, and HTTP redirects are disabled.
  - `--max-api-payment-usdc` overrides `X402_MAX_PRICE_USD` (default `1.00`);
    `none` or `off` removes the amount cap while keeping the Base USDC
    restriction.
- **CLI and environment precedence:** CLI > environment > config > default
  (`--env-file` values override inherited environment values, and `--env-file`
  applies to `serve`, `config check`, `catalog tools`, `catalog tags`, and
  standalone `catalog route` under `--config` or standalone `--provider`,
  except supervised qualification modes). Standalone flags include `--spec`,
  `--provider`, `--base-url`, `--prefix`, `--include`, `--exclude`, `--tags`,
  `--exclude-tags`, `--read-timeout-seconds`, `--transport`, `--host`,
  `--port`, and `--bearer-token`. Supported `X402_MCP_GENERIC_*` variables are
  `SPEC`, `BASE_URL`, `PREFIX`, `NAME`, `PRICING_KEY`, `INCLUDE`, `EXCLUDE`,
  `TAGS`, `EXCLUDE_TAGS`, `INSTRUCTIONS_TEXT`, `HELP_URL`, and
  `MAX_DESCRIPTION_CHARS`.
- **Listing and pagination:** `tools/list` returns the complete advertised
  inventory in one response without `nextCursor`. `tools_search` defaults to 25
  per page (any positive `limit` accepted), with cursors bound to the listener
  and catalog snapshot. Local bundled-provider browse defaults to 25 per page
  with no local ceiling. See the
  [remote directory search limits](agent-sources.md#external-directory-matching).

## Operation URL extensions

Set `operation_url_extension = "x-direct-url"` on a provider or source to read
absolute public HTTPS request URLs from that operation-level OpenAPI extension:

- URLs must be public HTTPS without credentials or fragments and with a fixed
  authority; path parameters substitute normally and fixed query strings stay
  intact. Absent extensions fall back to `base_url` with the OpenAPI path;
  invalid values fail catalog construction for that operation.
- The original OpenAPI path still controls tool naming, filters, and
  `response_mappings`. Generic dynamic imports ignore URL extensions; bundled
  imports preserve them. HTTP redirects remain disabled.
- Cover traffic profiles stay bound to `base_url`'s origin; calling a different
  operation URL origin logs a warning and proceeds without cover (disable cover
  on multi-origin providers such as Lonestar). See
  [cover configuration](cover-traffic.md#enable-and-inspect).

## OpenAPI inputs and request bodies

- **Parameter merging and collision naming:** Path, query, body, and header
  inputs keep separate identities. Operation parameters override inherited
  path-level parameters at the same location and name (headers compare
  case-insensitively). Cross-location name collisions append `_query`, `_body`,
  or `_header` (plus numeric suffixes when needed) while keeping the original
  wire name.
- **Flat vs. whole-body JSON:** Required simple object bodies flatten into
  top-level tool arguments (including pure required-only `oneOf`, `anyOf`, and
  `allOf` constraints); set `additional_properties = false` (or `true`) on a
  provider or source to control `additionalProperties` on the flat outer tool
  schema. A closed flattened body stays closed with `additional_properties =
  true`. Optional bodies, property-bearing `allOf`, exclusive or closed
  alternatives, and unfamiliar root constraints wrap under a single `body`
  argument (or `body_body` on collision) with a closed outer schema so callers
  can omit the body, pass `{}`, or pass JSON `null` distinctly.
  [Parameter description overrides](#observed-prices-and-description-overrides)
- **Media types:** The encoder prefers representable `application/json`, then a
  concrete `+json` type ([RFC 6839](https://www.rfc-editor.org/rfc/rfc6839.html#section-3.1)),
  requiring UTF-8. Form, multipart, XML, and binary-only bodies are omitted
  explicitly rather than exposed as empty JSON.
- **Scalar and union parameter schemas:** Path inputs serialize scalars; query
  inputs serialize scalars or repeated scalar arrays (`explode = true`).
  Composed parameter schemas need an explicit root wire type except two-arm
  query `anyOf` scalar unions (with at least one non-null arm; `null` is
  omitted) and `string | string[]` query unions. Null-only inputs and object,
  array, and null-only query array items are omitted as unsupported.
- **OpenAPI 3.1 `$ref` and recursion:** OpenAPI 3.1 (and unspecified-version)
  `$ref` siblings preserve the seven JSON Schema 2020-12 basic annotations
  (`title`, `description`, `default`, `examples`, `deprecated`, `readOnly`,
  `writeOnly`) via `allOf`; assertion-bearing or unknown siblings omit the
  operation with `schema_reference_siblings`. Exact unconstrained recursive
  all-JSON-types unions normalize to an unconstrained schema (preserving
  description) under standard JSON Schema 2020-12 dialects. Used `$ref` targets
  under string `$id` scopes, `$dynamicRef`, and `$recursiveRef` omit the
  operation with `input_reference`.
- **Omission handling:** Serving and dynamic registration keep supported
  siblings, log a `WARN` summary, and append omission notices to MCP
  instructions. Expected unsupported operations should be excluded via
  `exclude_operations` and documented in
  [provider caveats](../providers/CAVEATS.md). Strict inspection and
  qualification reject partial catalogs (standalone `catalog tags` inspects raw
  tags without compatibility validation).

## OpenAPI header parameters and idempotency

OpenAPI `in: header` business parameters become tool arguments preserving their
declared spelling (adding `_header` on cross-location collision). Required
headers must be supplied before dispatch; absent or null optional headers are
omitted.

- **Serialization:** Supports OpenAPI `simple` style (scalars as text, arrays
  comma-joined, flat objects as `key,value` or `key=value` with
  `explode = true`). Unsupported styles, content-based headers, nested values,
  and invalid header names or values fail without logging header values.
- **Ignored and forbidden headers:** `Accept`, `Content-Type`, `Authorization`,
  and x402 payment headers (`PAYMENT-SIGNATURE`, `X-PAYMENT`,
  `PAYMENT-REQUIRED`, `PAYMENT-RESPONSE`, `X-PAYMENT-RESPONSE`) are ignored
  case-insensitively and never become tool arguments. Cookies and framing,
  connection, or proxy headers are rejected. Header sets exceeding the HTTP
  library's capacity fail cleanly rather than panicking.
- **Automatic `Idempotency-Key`:** Any OpenAPI `Idempotency-Key` header is
  managed automatically and hidden from tool arguments. Each tool invocation
  generates a fresh random UUID string and reuses it across unsigned retries,
  signing, and recognized insufficient-funds wallet traversal (two separate
  calls generate two different keys). Catalog construction accepts
  unconstrained strings, `format = "uuid"`, and length bounds admitting 36
  characters, and rejects incompatible formats. Interrupted calls are not
  replayed after restart. See [manual raw headers](agent-sources.md#manual-raw-headers)
  for `raw_call`, which does not synthesize `Idempotency-Key`.

## Optional agent API discovery

Enable dynamic API discovery per authenticated listener with
`provider_management_local`, `provider_management_remote`, and `raw_url_calls`
(all default `false`), an explicit `wallet`, optional
[`remote_provider_directories`](agent-sources.md#external-directory-matching),
and optional `source_registry_file` for persistence.

| Arrangement | Configuration | Payment identity sharing |
| --- | --- | --- |
| All agent-added APIs share one wallet | Each listener sets `wallet = "agent_shared"` | Across APIs and participating listeners |
| Listener-specific wallets | Each listener names its own `wallet` | Across added APIs on that listener |
| Operator-defined listener groups | Several listeners reference one `wallet` | Within that group |

- Tool counts do not gate registration or refresh. A `tools/list` response with
  more than 500 advertised definitions logs an advisory `WARN` suggesting
  `discover_on_demand = true`.
- `providers/x402-list.toml` and `providers/agents-trust.toml` expose those
  APIs as regular providers. Directory search uses the listener's
  `remote_provider_directories` choice independently.
- See [agent source management](agent-sources.md) for the complete tool table,
  directory matching, URL discovery, raw headers, limits, and `sources` CLI
  maintenance.

## Provider reliability observations

Provider files may declare `reliability_tags` and a `reliability_note` to warn
at serving startup (unrelated to OpenAPI `tags`). Tags never hide tools, block
payments, or enable retries.

| Tag | Startup warning |
| --- | --- |
| `upstream_timeout` | Provider reported upstream timeouts, possibly inside a 200 response. |
| `upstream_rate_limited` | An upstream dependency rate-limited the provider. |
| `intermittent_response_body` | Response delivery failed after payment; do not auto-replay. |
| `catalog_http_403` | Repeated catalog HTTP 403 responses can prevent live catalog startup. |
| `intermittent_catalog_connection` | Catalog connections have intermittently timed out. |
| `intermittent_help_connection` | Help retrieval has intermittently timed out. |
| `degraded_upstream_data` | Successful responses have contained failed or missing upstream inputs. |
| `questionable_result_relevance` | Result metadata did not match the query; check relevance. |
| `slow_pricing` | External pricing document retrieval has been slow or intermittent. |

Serving emits one stderr warning per listener-bound source and tag before
fetching its catalog (standalone serving warns too; inspection commands stay
quiet, and `config show` prints the tags). Lists replace on inheritance
(`reliability_tags = []` suppresses startup warnings without affecting runtime
failure warnings). Unknown tags fail validation. Put individual operation bugs
in [provider caveats](../providers/CAVEATS.md) rather than provider-wide tags.

## SocialFetch and tag selection

`providers/socialfetch.toml` loads SocialFetch's OpenAPI tags, excludes `Auth`,
account and monitor ownership, webhooks, and `GET /health`, and calculates
credit-to-USDC estimates via `credit_pricing`.

```sh
target/debug/x402_treazury catalog tags --provider providers/socialfetch.toml
target/debug/x402_treazury catalog tools --provider providers/socialfetch.toml --tags Twitter,YouTube
```

- `tags` and `exclude_tags` match exact case-sensitive OpenAPI tags (`exclude_tags`
  wins; lists replace on inheritance and `[]` clears; untagged operations never
  match `tags`).
- `catalog tags` prints raw JSON tag counts without loading keys, help, or
  pricing (standalone mode tolerates unsupported operations; deployment mode
  uses strict catalog loading). Pass `--spec tests/fixtures/socialfetch_openapi.json`
  for a scoped offline preview.
- SocialFetch's `include = ["/v1", ""]` strips `/v1` from generated names
  (`socialfetch_twitter_profiles_handle`) while keeping `/v2` prefixes
  (`socialfetch_v2_linkedin_*`) without altering HTTP paths.

## Optional Tor routing

Direct networking is the default. Add `[network]` to route all outbound HTTP
and gRPC traffic through a running Tor SOCKS listener:

```toml
[network]
mode = "tor"
socks_endpoint = "127.0.0.1:9150"
isolation_namespace = "x402_treazury"
socks_auth = "tor_extended"
connect_timeout_seconds = 120
read_timeout_seconds = 60
tor_isolation_retries = 2
```

- **`socks_endpoint`:** Nonzero `host:port` (IP or hostname, such as `tor:9050`)
  or Unix socket `unix:/absolute/path`. If unreachable during serving startup,
  `serve` logs `ERROR` (repeating every minute while waiting), retries every 5
  seconds, and resumes automatically once reachable without blocking ready
  listeners or cached catalogs.
- **`socks_auth`:** `"tor_extended"` (default, username `<torS0X>0` with
  namespace and token in password) or `"legacy"` (username `x402_treazury` with
  derived password). Tokens are deterministic SHA-256 hashes of
  `isolation_namespace` and identity; use distinct namespaces to separate
  deployments. Tor resolves DNS remotely, validates TLS, ignores ambient
  proxy environment variables, and never falls back to direct connections.
- **`tor_isolation_retries`:** Default `2` fresh Tor isolation scopes after the
  initial unsigned attempt (`0` allows one attempt). Direct mode and
  `catalog warm --direct` use one origin attempt (Base direct transport
  recovery stays two attempts). Applies only to unsigned reads, never signed
  requests. See [retry ownership](network-egress.md#retry-ownership).
- **Timeouts:** `read_timeout_seconds` defaults to `60` in both modes (range
  `1..=86400` at network level; positive floats up to `86400` on providers, with
  a `1ms` floor in standalone mode) and measures inactivity per read rather than
  total download time. `connect_timeout_seconds` defaults to `15` direct and
  `120` Tor (range `1..=300`). Base RPC reads use these same timeouts with no
  separate total deadline.
- **`--network-config FILE`:** Overrides `[network]` (and `[relay_fallback]` if
  present) for `serve`, `catalog`, `config`, `sources refresh`, and `wallet`
  commands:

```sh
x402_treazury catalog tools --provider providers/socialfetch.toml \
  --network-config examples/network/tor.toml
x402_treazury wallet init --state-dir state/public-demo \
  --network-config examples/network/tor.toml
```

  Qualification snapshots and supervised serving reject `--network-config`;
  set `[network]` in their pinned deployment file before preparation.
- **Identity separation:** Payments, Base balance checks, and NEAR funding use
  the EVM address; treasury sync uses the treasury UUID; catalogs and help use
  discovery-origin identities. See [network egress](network-egress.md).

## Creating offline OpenAPI fixtures

The `snapshot_spec` example converts a local JSON file or HTTP(S) OpenAPI URL
into a compact, sorted-key request-only fixture (stripping responses and
`components.responses` while keeping request schemas, tags, descriptions, and
extensions). The destination directory must exist:

```sh
cargo run --locked --example snapshot_spec -- \
  https://www.socialfetch.dev/openapi.json /tmp/socialfetch-requests.json

cargo test --locked --example snapshot_spec
```

## Provider HTTP and TLS policy

Provider HTTPS traffic (catalogs, help, and API requests, including
cross-origin operation URLs) requires **HTTPS, HTTP/2, and TLS 1.3** by
default. Cleartext URLs are rejected before I/O or signing (local spec files
remain supported). Override per provider or source only when needed:

```toml
allow_http1 = true  # Permit HTTP/1.1 fallback while still offering HTTP/2.
allow_tls12 = true  # Permit TLS 1.2 while still preferring TLS 1.3.
```

Both default to `false`, inherit via `extends`, never relax certificate or
hostname checks, and never trigger automatic downgrade retries. Generic
agent-added providers keep the strict defaults.

- **Private trust anchors:** Add PEM certificate files via
  `[network].trusted_ca_files` (absolute paths; extends both HTTP and gRPC
  trust stores; missing or malformed files fail startup; requires restart to
  reload; preserved by `catalog warm --direct`):

```toml
[network]
trusted_ca_files = ["/absolute/path/private-ca.pem"]
```

- **Connection pooling and telemetry:** Concurrent requests multiplex over
  HTTP/2 only when origin, identity, timeout, and transport flags match.
  Rotating wallets never reuse a prior wallet's TLS connection. `DEBUG` logs
  record negotiated `http_version` and request stage without URLs or
  credentials.
- **Cover traffic:** Enabled by default in Tor mode and off in direct mode. Set
  `network.cover_traffic_enabled = false` globally or
  `cover_traffic_enabled = false` per provider or source to opt out. See
  [cover traffic](cover-traffic.md).

## Provider payment version

Set `x402_version = 1` or `x402_version = 2` in a provider TOML or deployment
source to require that version on paid API calls. Omit the setting to use
normal negotiation.

- **Version 1:** Uses a version 1 challenge from the response header or JSON
  body. Two version 1 copies must carry identical offers. A header advertising
  version 2 can accompany a version 1 body, as in
  [Carredge](../providers/CAVEATS.md#carredgetoml--carredge).
- **Version 2:** Requires a version 2 `PAYMENT-REQUIRED` header. The v1 body
  cannot supply a payment when the v2 challenge fails.

The setting applies before signing and inherits through `extends`. A
registered bundled provider retains its version. Raw calls and generic
OpenAPI imports use normal negotiation. An explicitly configured relay adapter
uses its own provider's version when paying its fee.
Payment errors distinguish duplicate challenge headers, conflicting version 1
offers, and a missing configured version without returning seller text.

## Pricing metadata and runtime hints

Startup, inspection, and warming never send API calls to probe prices; tool
descriptions use authored metadata, and live calls still obtain and cap their
own x402 challenge.

### Runtime price hints and MCP offer metadata

- **Selection order:** Before the first unsigned request, managed selection
  checks a fresh observed offer first, then a valid advertised per-call USDC
  estimate, then unknown pricing.
- **Shared hint cache:** Validated managed challenges update estimates on the
  remote catalog cache entry, shared across compatible aliases, listeners, and
  wallet groups without sharing signers or transports. Hints expire with the
  parent catalog (`catalog_cache_ttl_seconds`).
- **Per-call MCP pricing block:** Managed API results append a second MCP JSON
  text block (`{"x402_pricing":{"amount":"...","currency":"USDC","basis":"observed_offer"}}`)
  on both successes and post-validation errors (omitted when no target offer
  was validated or for static SDK signing). Relay fees never supply a target
  offer. See [output boundaries](architecture.md#mcp-and-output-boundaries).

### Advertised pricing configuration

Configure embedded catalog pricing or an external pricing document under
`[pricing]`:

```toml
[pricing]
source = "catalog"
format = "payment-info"
key = "x-payment-info" # Optional; default for payment-info.
unit = "call" # Fallback unit when metadata omits one ("call" or "request").
usd_as_usdc = true # Opt in to treating reviewed USD tariffs as 1:1 USDC hints.
```

- **Catalog formats:** `payment-info` reads structured operation metadata
  (`pricing_key` is shorthand for `source = "catalog"`,
  `format = "payment-info"`). `x402` defaults to `x-x402` and parses
  dollar-prefixed `price` strings. A local `pricing` or `pricing_key` replaces
  both inherited pricing settings as one unit.
  A catalog with a top-level `operations` array can supply a structured
  `pricing` field on each operation. The compiler reads it as `payment-info`
  by default. The configured key selects an OpenAPI field. An operations
  digest uses `pricing`. Selecting an external source takes precedence.
- **External formats:** Set `source` to an HTTPS URL with
  `format = "x402-resources"` (`resources` array with explicit HTTP methods) or
  `format = "x402-discovery"` (`items` array; may omit method when one catalog
  operation matches). External pricing loads asynchronously during serving
  (sharing Tor `catalog_concurrency` slots and relay fallback), never delays
  tool publication, and is skipped by inspection and qualification. Refreshing
  a catalog re-downloads its external pricing document and binds its expiry to
  the parent catalog.
- **Catalog selection estimates:** A fixed per-call or per-request price can
  supply a selection estimate in Base USDC. The amount must be nonnegative
  with at most six decimal places. Enable `usd_as_usdc` to use a fixed USD
  price as the estimate. Explicit units take precedence over the configured
  fallback. Catalog prices with missing or malformed units, ranges, and
  dynamic, metered, credit, or unsupported-asset tariffs remain display
  evidence. When `protocols[].x402.networks` lists network alternatives, one
  must support Base USDC. A scheme declared under `protocols[].x402` must be
  `exact`.
- **Nested tariff terms:** The selected price block and its enclosing tariff
  objects supply billing terms. The nearest declaration supplies the display
  mode and unit. Any nonfixed mode or explicit unit other than `call` or
  `request` along that path prevents using the tariff as a selection estimate
  for a paid request. Malformed declarations also prevent an estimate. This
  applies to both `mode` and `pricingMode` and to embedded and external
  prices. A matching external offer remains visible as a sample.
- **External selection estimates:** A matched external record can supply a
  compatible exact Base USDC offer for one request. An offer can supply a
  selection estimate when the tariff is fixed per call or request, or when
  the offer is the only price. If the record and configuration omit the unit,
  the offer itself supplies the per-request unit. Alongside a range, a dynamic
  or metered tariff, or an unrecognized tariff such as a percentage fee, the
  renderer labels the offer as a sample. Explicit non-call or malformed units
  also keep the offer as a sample. The description reports when published
  offers supply no exact Base USDC amount.
- **Free operations:** An explicit `authMode = "free"` supplies a zero estimate
  without an amount, currency, or unit. A fixed zero amount can agree with
  that declaration, and the description retains access conditions. Conflicting
  free and paid declarations leave the estimate unknown.
- **Description suffixes:** Published amounts appear as
  `Listed: ~$0.015/call [spec].`, `Listed: from ~$0.01/call [spec, dynamic].`,
  or `Listed: free [spec].`. A missing billing unit appears as `[spec, no unit]`
  or alongside the external source label. Generated USD and USDC amounts use
  `$`. `Cost: unknown.` appears when no pricing fragment is available.
  Server instructions include the pricing caveat once.
  > Prices are estimates or sampled payment offers, not guaranteed quotes. Costs may vary with arguments; metered charges may be below the displayed maximum. Unknown does not mean free.

See the [custom pricing adapters plan](plans/custom-pricing-adapters.md) for
unhandled formats and the [provider pricing review](provider-updates.md#pricing-evidence-and-variation-review)
for maintaining advertised evidence.

### Credit tariff conversion

Providers with structured credit tariffs can display approximate USDC costs:

```toml
pricing_key = "x-socialfetch-credits-pricing" # Read vendor billing terms.
[credit_pricing]
credit_cost_key = "x-socialfetch-pricing"
usdc_per_credit = "0.014"
```

Conversion uses exact integer arithmetic with a positive rate of at most six
decimal places. It supplies display estimates. The converter supports
`baseCredits`, `maxCredits`, `normalizationFailureCredits`, `surcharges`
(including conditions that check for a value in an array), URL batches,
per-record metering, and `billableDomainErrors`. It accepts `x402ExactPolicy`
as transport metadata. A fixed estimate appears
as `Estimated cost: $0.014 on success.` when it replaces a matching
fixed-credit tariff for successful requests. Metering, surcharges, and
failure charges remain visible. Malformed metadata leaves vendor
terms in place and logs one `WARN` summary per catalog build. `DEBUG` logs
per-tool failure reasons without raw metadata.
SocialFetch uses `0.014` USDC per credit as a fixed display estimate.

### Observed prices and description overrides

MCP listing, definition, and search results can show published pricing and
configured terms alongside a cached offer from an earlier request, such as
`Sample offer: $0.005 for one request.` The amount can vary with arguments.
The live description drops cached pricing when it expires or the source
retires. CLI output and search ranking use the compiled description. External
prices loaded in the background and recorded offers leave search cursors stable.

`description` replaces operation prose. `pricing_description` adds billing
terms alongside the published tariff. When a note starts with a copy of the
published tariff, including a recognized legacy format, the description keeps
one copy and preserves the rest of the note. A unitless tariff leaves the
authored per-call price visible. An eligible per-request offer can still match
a legacy offer note.

Plain billing notes appear directly in the description. A separate configured
dollar amount carries a `Configured pricing:` label alongside a published
tariff, credit estimate, or sample offer.

Use `description` to edit help text. Serving discards `pricing_description`
on a help tool, and strict inspection rejects that override.

```toml
[overrides.example_search]
description = "Search records. Returns matching records and their sources."
pricing_description = "Billed per returned record. Empty results are free."

[overrides.example_search.params]
query = "Terms to find in the record title."
```

Parameter notes target the exposed argument first, then an exact member of
the whole-body argument, then the legacy `NAME_body` spelling. If two notes
target the same field, serving uses the exact spelling and reports the
discarded legacy note. Set the inherited legacy key to `false` in
`overrides_patch` to remove it. The provider comparison reports competing matches,
selected schema paths, malformed override tables and notes, and missing
targets. Serving reports unknown override fields and applies valid fields.
Strict inspection rejects discarded edits even if a tool-name
filter hides the compiled tool.

Descriptions have no character limit by default. Set `max_description_chars`
in TOML or `X402_MCP_GENERIC_MAX_DESCRIPTION_CHARS` for standalone serving.
The limit counts Unicode scalar values across the final description,
including authored text, help tool descriptions, and live pricing. A letter
and a separate combining accent count as two values. If the limit is `N`
and the description is longer, the renderer keeps the first `N` values and
appends a newline and `[truncated by max_description_chars=N]`. The newline
and marker are outside the limit. Compilation-time truncation logs a `WARN`
with the tool name, setting, and character limit.

## Automatic discovery disk cache

Deployments with an existing `treasury.state_dir` automatically cache remote
catalogs in `http-cache/discovery-v1.sqlite` without opening wallet keys or
creating directories. Standalone providers, local specs, and dynamic imports do
not use this disk cache. Opt out per provider or source with
`http_cache_enabled = false`.

```toml
catalog_cache_ttl_seconds = "indefinite" # Default; 0 disables disk caching; positive seconds set expiry.
```

- **Lifetime and invalidation:** Catalogs and their cached price hints share
  one lifetime starting when the download completes. Local TTL overrides
  upstream `Cache-Control` and `Expires` (including `no-store` and `max-age=0`).
  Reads and price observations never extend expiry. Expiry, forced refresh,
  changed bytes, or source config edits discard cached price hints together.
- **Revalidation and staging:** Expired entries with `ETag` or `Last-Modified`
  use one conditional GET (HTTP `304` reuses the body and starts a new TTL).
  Downloaded or revalidated bytes are staged and committed only after
  generation and `send_tools` validation pass (failed refreshes keep the old
  snapshot on disk while returning the refresh error).
- **Private cache rules and bounds:** Permits `private` and `Set-Cookie`
  responses (cookies are never saved or sent) and supports `Vary` by pinning
  the unsigned discovery header profile (`Vary: *`, conditional variants,
  credentialed URLs, and relayed `Vary` responses are not stored). Keys
  separate URL, network and isolation policy, transport flags, timeouts,
  limits, TTL, and resource kind. Caps: 4,096 entries, 128 MiB payload and
  metadata, 144 MiB SQLite file (oldest entries evict with a log). Cache I/O
  errors warn and fall back to origin fetches. Treat `http-cache/` as private
  cache state; stop the app before deleting it.
- **Bypass and logging:** Live qualification always bypasses disk caching.
  `DEBUG` logs report `miss`, `disk_hit`, `revalidate`, `revalidated`,
  `shared_load`, and `stored` along with response policy labels without raw
  headers.

### Runtime source refresh

Deployment sources and dynamic registrations automatically coalesce a
background refresh on a final unsigned target HTTP `404`, `405`, or `410` after
normal retries and relay fallback finish (standalone `serve --provider` loads
once at startup). Signed target outcomes, relay service errors, `400` and `422`,
and price changes do not trigger refresh.

- Existing tools and price hints stay callable while refresh runs and if
  refresh fails. Accepted calls keep their active definitions and payers.
  Refresh never allocates wallets or starts funding.
- If a replacement catalog selects an uninitialized static wallet, publication
  fails and existing tools stay active until restart. If all operations become
  unsupported, the source publishes an empty inventory with omission notices
  while healthy siblings stay available.
- Replacement definitions publish immediately without waiting for optional
  pricing; old handles and hints retire before the replacement's pricing task
  starts so retired work cannot overwrite new metadata. Refresh failures do not
  auto-retry until a later fresh `404`, `405`, or `410` arrives. Static MCP
  clients that cache `tools/list` may need to reconnect after a schema change.

### Warming a selected source, including directly for a Tor deployment

```sh
x402_treazury catalog warm --config deployment.toml --source provider_id
x402_treazury catalog warm --config deployment.toml --source provider_id --direct
```

- `--source` takes deployment source IDs (repeatable or comma-separated; omit
  for all sources). Selected remote catalogs and external pricing documents
  download unconditionally, requiring an existing `treasury.state_dir` and
  rejecting `http_cache_enabled = false` or `catalog_cache_ttl_seconds = 0` on
  selected sources.
- Warming starts no listeners or funding workers and opens discovery wallets
  only when paid relay warming is enabled. A running server keeps its in-memory
  snapshot until runtime refresh or restart.
- `--direct` fetches unsigned discovery documents over direct networking
  (exposing your direct IP to those providers) without changing the deployment
  file. Direct-warm entries carry separate provenance bound to the original
  network policy; serving reuses them while fresh (with a stderr warning) and
  never sends directly fetched validators over Tor. Once expired, normal
  loading returns to the configured network.
- Prints one JSON summary (`direct`, `outcome: "ok"` or `"degraded"`, and
  per-source status), exiting `0` for both completed outcomes and nonzero on
  local config, state, cancellation, or internal errors. See
  [framework integration](framework-integration.md) for the full JSON schema.

## Relay fallback

Deployments can configure ordered paid relay fallback for catalogs, document
reads, directory searches, and opt-in API requests, either inline or via
`--network-config`:

```toml
[relay_fallback]
source_discovery = true   # Remote directory searches and service details (default true).
provider_metadata = true  # Catalogs, help, imports, and URL probes (default true).
api_requests = false      # Generated API tools and raw_call (default false).
# wallet = "web"          # Optional static payer override when [wallet_bootstrap] is absent.
# sources = ["social"]    # Optional static source filter; empty selects all.

[relay_fallback.relays]
GET = ["../../providers/readfetch.toml", "../../providers/curl/provider.toml", "../../providers/agproxy.toml"]
POST = ["../../providers/readfetch.toml", "../../providers/curl/provider.toml", "../../providers/agproxy.toml"]
HEAD = ["../../providers/readfetch.toml", "../../providers/curl/provider.toml", "../../providers/agproxy.toml"]
```

### Configuration and categories

- **Method lists:** Keys are `GET`, `HEAD`, and `POST` (matched by the original
  request method; lists do not inherit from `GET`). Each listed provider must
  declare `relay_format` (`"curl"`, `"agproxy"`, `"horizonpulse"`, or
  `"readfetch"`). Obsolete names (`discovery_relay`, `fallbacks`, single
  `provider`, `serve`, `warm`, `on_demand`, `search`) are rejected.
- **Local schema validation:** Runtime startup validates every relay adapter
  using its bundled local schema (`curl`, `agproxy-openapi.json`,
  `horizonpulse-openapi.json`, `readfetch-openapi.json`) with no remote fetches.
  See [provider caveats](../providers/CAVEATS.md) for adapter details.
- **Categories:** Defaults apply only when `[relay_fallback]` is present:

| Flag | Covered requests | Default |
| --- | --- | --- |
| `source_discovery` | Remote directory searches and x402 List service details | `true` |
| `provider_metadata` | Static catalogs and warming, dynamic imports, help, usage docs, and URL probes | `true` |
| `api_requests` | Generated API tools and `raw_call` | `false` |

### Origin sequence, traversal, and limits

- **Origin sequence:** Origin runs once in direct mode, or `1 + tor_isolation_retries`
  fresh Tor scopes, retrying eligible transport errors and HTTP `400`, `403`,
  `408`, `429`, `500`, `502`, `503`, and `504` (honoring `Retry-After` on `429`
  and `503`). Local policy, destination, and size rejections, unsolicited `206`
  on complete catalogs, and other `4xx` responses are terminal and never
  trigger paid fallback.
- **Relay traversal:** After origin fails, each configured provider for that
  method runs at most once (using the same Tor retry count for its unsigned
  relay-service leg). Adapters skip requests whose method, body, or required
  headers they cannot preserve before paying. A relay service failure,
  malformed envelope, or relay-service `402` advances to the next provider even
  after a possible charge; a decoded **target** `402` is terminal and never
  signed. Any prior signed target attempt suppresses API relay fallback.
- **Payer selection:** With `[wallet_bootstrap]`, metadata and directory relay
  fees always use `bootstrap_group`, and API and raw relays use the call's
  assigned group. Without `[wallet_bootstrap]`, runtime reads use the endpoint
  or source wallet, and static sources use `relay_fallback.wallet`, an explicit
  source wallet, or (for shared sources) the highest SHA-256 score of
  `["discovery-wallet-v1", source_id, wallet_name]` (reported by `config show`
  as `discovery_wallets`). Exclusion-only routed sources make no paid relay
  calls.
- **Serialization and caching:** Static paid fetches serialize per relay
  provider across wallets while keeping shared state locks brief. Compatible
  static aliases share one process acquisition and outcome. Relay catalogs use
  the private disk cache (rejecting relayed `Vary` responses), partitioned by
  adapter, position, payer, URL, and size bounds; expired entries return to
  origin-first loading. Outer relay-envelope and decoded target size limits are
  enforced separately.
- **Trust boundary:** Every relay connection uses `src/network.rs` (Tor never
  falls back to direct). The relay sees the target URL, headers, and plaintext
  response and may follow redirects remotely (every reported final URL and Curl
  hop is checked against the public-destination policy; AGProxy, Horizon Pulse,
  and ReadFetch report only the final URL). Never enable `api_requests = true`
  for authenticated or sensitive API payloads.
- **Warming and inspection:** `serve` binds listeners before waiting on relay
  payers; `catalog warm` (without `--direct`) opens resolved relay wallets
  without starting funding; `config check`, `catalog tools`, and
  `catalog warm --direct` never pay relays; qualification rejects paid relay
  startup.
