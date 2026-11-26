# Agent source management

Multi-server HTTP deployments can let agents discover, register, and call
public x402 APIs at runtime without restarting `x402_treazury`. All three
capabilities are off by default and configured independently per authenticated
listener.

## Configuration and authority

Enable provider management and raw URL calls on any deployment listener that
declares an explicit `wallet` (automatic wallet assignment cannot supply this
wallet):

```toml
[servers.toolbox]
listen = "127.0.0.1:3010"
bearer_token_env = "X402_TREAZURY_MCP_TOKEN"
wallet = "general"
sources = []
discover_on_demand = true
provider_management_local = true
provider_management_remote = true
remote_provider_directories = ["coinbase", "x402-list"]
raw_url_calls = true
source_registry_file = "../../state/toolbox-sources.sqlite"
```

| Setting | Default | What it enables |
| --- | --- | --- |
| `provider_management_local` | `false` | Offline bundled provider listing and registration by `provider_id` |
| `provider_management_remote` | `false` | Remote directory search, URL or slug details lookup, and public HTTPS OpenAPI registration |
| `remote_provider_directories` | `[]` | Ordered subset of `"coinbase"`, `"coinbase_curated"`, `"x402-list"`, and `"agents-trust"` for remote search |
| `raw_url_calls` | `false` | Direct calls to documented public HTTPS URLs via `x402_treazury_raw_call` |
| `source_registry_file` | omitted | Owner-only SQLite file for persisting registrations across restarts (ephemeral when omitted) |
| `discover_on_demand` | `false` | Advertise search and call wrappers in place of individual provider API schemas |

Authority boundaries:

- **Authenticated principal:** Each listener's bearer token defines its
  authority; agent arguments and session fields cannot choose wallets, owners,
  or persistence.
- **Endpoint isolation:** Dynamic registrations belong only to the listener
  that created them and cannot modify configured sources, wallet settings, or
  spending limits. Persistent listeners must each use a distinct
  `source_registry_file`; sharing one file across listeners is rejected.
- **No wallet allocation or funding:** Listing, searching, probing, and
  registering APIs never create wallet pools or trigger funding. Registered
  APIs share the listener's explicit wallet and its `max_api_payment_usdc` cap.
- **Wrapper visibility:** Enabling either `provider_management_local` or
  `provider_management_remote` automatically advertises
  `x402_treazury_tools_search` and `x402_treazury_tool_call` so clients can
  discover and invoke newly registered APIs even if they cache `tools/list`.
  Enabling only `raw_url_calls` does not advertise those wrappers or restore
  persisted dynamic sources.
- **Directory selection:** `coinbase` searches both curated and non-curated
  Coinbase resources; `coinbase_curated` restricts the same search to curated
  resources. Leaving `remote_provider_directories = []` with
  `provider_management_remote = true` omits `x402_treazury_providers_search_remote`
  and `x402_list_id` lookups while keeping `base_url`, `documentation_url`, and
  `providers_add_remote` available.
- **Obsolete keys:** The retired keys `source_management` and
  `sources_search_tool_instructions_text` are rejected at parse time.

## Agent tools

All built-in tools use the `x402_treazury_` prefix. Generated provider tool
names must use a different prefix. See the
[reserved tool names](configuration.md#catalog-failures-and-strict-inspection)
rules. This table shows the built-ins' short names.

| Tool | Enabled by | Purpose |
| --- | --- | --- |
| `tools_search` | `discover_on_demand` or either provider-management flag | Searches callable tools on this endpoint and returns full input schemas. Makes no network requests. |
| `tool_call` | `discover_on_demand` or either provider-management flag | Invokes a callable `tool_id` with `arguments`, applying normal payment caps and filters. |
| `providers_list_local` | `provider_management_local` | Lists or searches bundled provider definitions offline (`q`, `page`, `per_page`). |
| `providers_add_local` | `provider_management_local` | Registers a bundled provider by exact `provider_id` and optional `name`. |
| `providers_search_remote` | `provider_management_remote` plus nonempty `remote_provider_directories` | Queries every selected remote directory concurrently (`q`, `limit`, `max_price_usd`). |
| `provider_details_remote` | `provider_management_remote` | Checks bundled matches, then probes `base_url`, reads `documentation_url`, or looks up `x402_list_id` (requires `x402-list`). |
| `providers_add_remote` | `provider_management_remote` | Registers a public HTTPS OpenAPI 3 JSON specification by `spec_url` and optional `name`. |
| `raw_call` | `raw_url_calls` | Calls a documented public HTTPS URL (`method`, `url`, optional `query`, `headers`, and JSON `body`). |

Recommended agent workflow:

1. Search already callable tools with `x402_treazury_tools_search`.
2. Search bundled definitions offline with `x402_treazury_providers_list_local`
   before spending on remote directories, and add matches with
   `x402_treazury_providers_add_local`.
3. If no bundled provider covers the need, search remote directories with
   `x402_treazury_providers_search_remote`.
4. Inspect a candidate with `x402_treazury_provider_details_remote`
   (`x402_list_id`, `base_url`, or `documentation_url`) to find its OpenAPI
   spec or usage guide.
5. Register a discovered OpenAPI 3 spec with
   `x402_treazury_providers_add_remote`, or call a documented non-OpenAPI
   endpoint directly with `x402_treazury_raw_call`.
6. Use `x402_treazury_tools_search` and `x402_treazury_tool_call` to inspect
   and invoke the newly registered tools.

## Tool search matching and results

`x402_treazury_tools_search` searches only the current endpoint's callable tool
inventory (configured tools, enabled dynamic registrations, and enabled
direct-swap tools, after listener filters and duplicate-name suppression).

- **Ranking:** Nonblank queries split on Unicode alphanumeric tokens using
  multi-term OR matching (no stemming, wildcards, Boolean operators, or
  quoted-phrase syntax), rank tools with BM25 over tool names, descriptions,
  and parameter names, prioritize literal substring matches, and break ties
  deterministically by `tool_id`. Blank queries enumerate the inventory in
  `tool_id` order.
- **Filtering and pagination:** Optional `source_name` restricts results to one
  source. `limit` defaults to 25 (any positive integer is accepted). `cursor`
  binds to the listener and catalog snapshot; if a catalog refreshes or a new
  source is registered, restart pagination from the beginning.
- **Response fields:** Responses return `available` (total tools in the scoped
  inventory), `total` (number of tools matching the query), `next_cursor`,
  `catalog_generation`, full input schemas for the returned page, and a
  zero-match `hint` distinguishing `available > 0, total = 0` from
  `available = 0`.

## Listing and URL discovery

### Bundled provider listing

`x402_treazury_providers_list_local` runs offline against embedded metadata from
`providers/`. Nonblank `q` uses the same literal-priority BM25 ranking over
`provider_id`, display name, description, and `api_base`. Blank `q` lists in
`provider_id` order (`page` defaults to 1, `per_page` to 25). An absent
`api_base` means the provider takes its server URL from its OpenAPI spec.

### Remote provider details and usage documents

`x402_treazury_provider_details_remote` requires exactly one of `x402_list_id`,
`base_url`, or `documentation_url` and returns `provider_matches` alongside
`url_discovery` status and results when probing:

- **`x402_list_id`:** Takes an exact x402 List `slug` (available only when
  `x402-list` is in `remote_provider_directories`). Performs one directory
  lookup (potentially paid), checks exact bundled matches by origin and base
  path, and probes the listing's API base when no bundled match applies.
- **`base_url`:** Checks exact bundled matches by origin and base path first.
  When no bundled match exists, probes a fixed list of common OpenAPI and
  documentation paths (`/openapi.json`, `/swagger.json`, `/docs/openapi.json`,
  `/.well-known/openapi.json`, `/llms.txt`, `/llms-full.txt`, and related
  locations) using up to four concurrent validated GET requests under normal
  `max_spec_bytes` and `max_help_bytes` limits. Origin `401`, `404`, `410`, and
  target `402` probe responses never trigger a paid relay read. When probing
  from a resource URL with no documented base path, start with the origin root
  (`https://host/`) and keep in mind that specs hosted under a deeper subpath
  or separate host may require a documented base URL.
- **`documentation_url`:** Fetches one exact public HTTPS URL (preserving query
  strings) with no redirects, under `max_help_bytes` (4 MiB) and the network
  read-inactivity timeout. Accepts complete UTF-8 without NUL for `text/plain`,
  `text/markdown`, `text/x-markdown`, or no `Content-Type`; rejects HTTP `206`
  and `Content-Range` responses, leading HTML or doctype markup regardless of
  `Content-Type`, and HTML, JSON, YAML, XML, binary, and other MIME types with
  an explicit error rather than returning truncated or scraped markup.

Configured `relay_fallback` applies `source_discovery` to directory lookups and
`provider_metadata` to spec probes and usage-document reads.

## External directory matching

`x402_treazury_providers_search_remote` queries every configured directory
concurrently with `q`, `limit` (default 5), and optional `max_price_usd`.
The search accepts limits from 1 to 20, or from 1 to 5 when Agents Trust is
the only configured directory. When a Coinbase directory is configured, the
shared query limit is 400 characters.

- **Coinbase (`coinbase` and `coinbase_curated`):** Queries Coinbase's public
  resource directory unsigned for Base mainnet USDC `exact` offers.
  `max_price_usd` filters resource prices. An origin `402` or `206`, or a
  `Content-Range` header, ends the unsigned acquisition. A complete origin
  response with invalid JSON can trigger configured relay fallback.
- **x402 List (`x402-list`):** Queries `/best` for Base services with
  `prefer=balanced` and `require_verified=false`, filtering by minimum service
  price. Directory data from `x402-list.com` carries a CC BY 4.0 attribution
  requirement.
- **Agents Trust (`agents-trust`):** Queries `/recommend` for Base API offers
  with compact results. Requires a query of up to 300 characters containing at
  least two letters or digits. An Agents Trust-only listener rejects missing
  or invalid queries before dispatch. In a mixed search that passes shared
  input validation, an Agents Trust query error affects only its own entry.
  In a search across multiple directories, the Agents Trust request caps
  `limit` at five. `max_price_usd`
  filters recommended offers. A recommendation normally costs 0.01 USDC
  separately, under the listener's payment cap. A paid answer can arrive
  before settlement. See the
  [Agents Trust receipt caveat](../providers/CAVEATS.md#agents-trusttoml--agents-trust).

Responses return a map keyed by directory name (`coinbase`, `coinbase_curated`,
`x402-list`, `agents-trust`). Each successful entry contains the full upstream
JSON object. A direct or relayed HTTP `206` or successful response with
`Content-Range` returns `local_error` with any possible charge preserved.
Other directories can still return results.
Relay advisories and possible-payment notices sit outside individual
directory objects. Paid directories and configured `source_discovery` relay
fallbacks can authorize multiple payments per search. The tool returns each
directory's response directly and leaves any follow-up search to the caller.

### Registration matching and naming

- **Local registration (`providers_add_local`):** Looks up an exact bundled
  `provider_id` and defaults the source name to `provider_id`. It preserves the
  provider's compatibility flags, filters, timeouts, byte limits, operation URL
  extensions, and embedded schema, while disabling static disk caching
  (`http_cache_enabled`), cover traffic (`cover_traffic`), and recommendation
  filtering (`use_recommended_tools`).
- **Remote registration (`providers_add_remote`):** Fetches a public HTTPS
  OpenAPI 3 JSON specification. If the canonical `spec_url` (including query
  string) uniquely matches a bundled provider and no generic registration
  already owns that URL, it adopts the bundled definition (preserving its
  compatibility repairs) even when `provider_management_local` is off. If
  multiple bundled providers share that URL, it returns the bundled choices
  (when local management is enabled) or asks the operator to disambiguate.
  Otherwise it registers the spec generically using strict HTTP/2 and TLS 1.3
  defaults and defaults the source name to the specification hostname.
- **Names, filters, and idempotency:** Registration names are case-preserving,
  at most 64 ASCII letters, digits, underscores, or hyphens, cannot contain
  `__`, and must be unique on the endpoint across disabled registrations and
  effective static bindings (including pending `send_tools` targets;
  exclusion-only sources reserve no name). Registered tool names receive a
  `dyn_<name>__` prefix (with numeric collision suffixes if needed). Listener
  `include_tools` allowlists still filter dynamically registered tools, so
  endpoints using `include_tools` must include `dyn_*` (or `dyn_<name>__*`) to
  expose dynamic registrations. Repeating a registration for the same
  `provider_id` or `spec_url` returns the existing registration without
  refreshing it. Concurrent registrations on the same endpoint share one import
  slot and publish complete snapshots atomically.
- Registered providers use bundled descriptions and instructions as authored.

## Manual raw headers

`x402_treazury_raw_call` accepts `method`, public HTTPS `url`, optional
`query`, optional string-valued `headers`, and optional JSON `body` (omitting
`body` and passing JSON `null` are distinct). It uses the listener's wallet,
payment cap, `max_response_bytes` default (16 MiB), network read timeout, and
public-destination policy, and is unaffected by tool-name filters.

Rules for caller-supplied `headers`:

- **Reserved and duplicate headers rejected:** `Accept`, `Content-Type`,
  framing, connection, proxy, cookie, and x402 payment headers
  (`PAYMENT-SIGNATURE`, `X-PAYMENT`, `PAYMENT-REQUIRED`, `PAYMENT-RESPONSE`,
  `X-PAYMENT-RESPONSE`), as well as case-insensitive duplicate header names,
  are rejected before I/O. Header values are never included in validation errors
  or logs.
- **Manual `Authorization`:** Allowed in `raw_call` only as an explicit
  caller-supplied header; it is never stored, registered, or injected into
  generated OpenAPI tools.
- **No relay, price handoff, or cross-payer handoff:** Supplying any nonempty
  `headers` map disables paid relay fallback, price handoff, and cross-payer
  insufficient-funds handoff so caller headers (which may carry credentials or
  vendor idempotency keys) are never forwarded to third-party relays or
  replayed across wallets. Manual-header writes also drop status- and
  body-driven POST retries and recover only from pre-dispatch TLS
  connection-establishment failures; see
  [unsigned x402 retries](architecture.md#unsigned-x402-requests-and-post-retries).
- **No automatic idempotency:** Unlike generated OpenAPI tools, `raw_call` does
  not synthesize or remember `Idempotency-Key` headers; the caller owns any
  idempotency header it passes.
- **Logging, error-echo, and linkability limits:** Unmodified `rmcp` can log
  raw MCP tool arguments (including `Authorization` headers) at `DEBUG` or
  `TRACE`, upstream HTTP responses and error bodies are not redacted and may
  echo credentials, and resending the same manual credential across wallet
  rotations can link wallet identities.

## Limits and network policy

Dynamic discovery, registration, probes, usage reads, and raw calls all use the
centralized factory in `src/network.rs`:

- **Public HTTPS only:** Non-HTTPS URLs, URLs with embedded credentials or
  fragments, loopback, private, and link-local destinations, and HTTP redirects
  are rejected before I/O or signing. Tor mode relies on `socks5h` remote DNS
  and Tor's `ClientRejectInternalAddresses` to reject private DNS answers.
- **Strict transport for generic imports:** Remote generic registrations and
  `raw_call` require HTTP/2 and TLS 1.3; agents cannot enable `allow_http1` or
  `allow_tls12`. Bundled imports retain their reviewed provider transport
  flags.
- **Origin routing:** Generic imports enforce same-origin API routing and
  reject undeclared cross-origin routes and cross-origin help URLs. Bundled
  imports keep their reviewed help origin and declared `operation_url_extension`
  public HTTPS targets.
- **Byte, schema, and registry limits:** Management tool requests cap at 64 KiB.
  Specs use `max_spec_bytes` (32 MiB) and `document_complexity_limit` (depth 64,
  500,000 nodes, 64 MiB traversal work); dynamic catalogs also cap at 10,000
  operations, 256 KiB per generated tool schema, 64 bytes per source name, and
  512 bytes per tool name, and reject external, unresolved, or cyclic `$ref`s
  and per-path or per-operation server overrides. Accepted live dynamic
  documents cap at 128 MiB in memory, and serialized SQLite registries cap at
  256 MiB. Usage documents use `max_help_bytes` (4 MiB) and API responses use
  `max_response_bytes` (16 MiB); bundled imports keep their configured provider
  limits.
- **Timeouts:** Bundled imports preserve the provider's `read_timeout_seconds`
  (falling back to `[network].read_timeout_seconds`, 60 seconds). Generic
  imports, `raw_call`, and `documentation_url` reads inherit
  `[network].read_timeout_seconds` directly with no source-management override.
- **Tor isolation:** In Tor mode, directory searches, spec fetches, and probes
  use discovery-origin identities (or the pinned wallet identity when a paid
  relay or directory fee applies), with no direct fallback.

## Persistence and operator maintenance

When `source_registry_file` is set on a listener, registrations are stored in
an owner-only SQLite database separate from wallet state (no wallet encryption
key is needed). Each record stores the owning listener name, source UUID,
revision, readable name, canonical `provider_id` or `spec_url`, accepted schema
bytes, and (for bundled imports) the accepted provider configuration.

On startup, a listener restores its persisted registrations only when at least
one of `provider_management_local` or `provider_management_remote` is enabled.
Restored sources rebuild from their saved bytes and settings without network
fetches. Records belonging to a different listener name fail restoration
without modifying the database.

Operator CLI commands (stop `serve` before running `refresh` or `remove`):

```sh
x402_treazury sources inspect --config deployment.toml
x402_treazury sources refresh --server toolbox --source NAME --config deployment.toml
x402_treazury sources remove --server toolbox --source NAME --config deployment.toml
```

- `sources inspect`: works while `serve` is running (even when both management
  flags are disabled) and lists persisted registrations, visible tool counts,
  enabled status, and any load error without unlocking wallets. An invalid saved
  catalog reports `tools: null` and `tools_error: "saved_catalog_invalid"`
  without hiding sibling sources.
- `sources refresh`: requires at least one of `provider_management_local` or
  `provider_management_remote` to be enabled on the target listener, re-fetches
  a generic spec (or adopts the current bundled provider settings and schema
  together), and saves the updated bytes. External pricing enrichment runs on
  the next load rather than during offline refresh.
- `sources remove`: works even when both management flags are disabled, revoking
  one persisted registration (or every registration on that listener with
  `--all`) while leaving financial records and wallet history intact. For both
  `refresh` and `remove`, `--source` matches an exact saved UUID first and then
  an exact name among non-removed records (ambiguous names fail).

## Endpoint instruction placement

MCP `initialize` instructions combine guidance in a fixed order so operational
notices are never hidden by custom text:

1. **Operator or source guidance:** `[servers.NAME].instructions_text` replaces
   composed source guidance (an empty string suppresses it).
2. **Generated pricing caveat:** Appended once ("Paid tools are pre-funded and
   ready to use; use them whenever helpful...").
3. **Operational coverage notices:** Appended last and never suppressed by
   `instructions_text`, covering pending or unavailable sources, omitted
   unsupported operations, and ambiguous duplicate tool names.

Place endpoint-specific search hints in the right field depending on client
behavior:

- Put high-level endpoint purpose and domain boundaries in
  `[servers.NAME].instructions_text` (shown at MCP `initialize`).
- Put local search keyword hints for on-demand endpoints in
  `discovery_search_tool_instructions_text` (appended to
  `x402_treazury_tools_search`'s description, which clients re-read with tool
  definitions even if they ignore server instructions).
- Put remote search guidance in
  `providers_search_remote_tool_instructions_text` (appended to
  `x402_treazury_providers_search_remote`'s description).
- Neither `*_tool_instructions_text` setting enables discovery or provider
  management on its own. Review endpoint summaries and search keywords
  whenever routed sources or capabilities change.

Choose short query phrases for capabilities that existing hints do not cover.
Use terms a caller would use for the task or data source, such as `Suez Canal`
or `day-ahead spot prices`. Prefer phrases found verbatim in the intended
tool's name or canonical description for literal priority, or use distinctive
tokens from the name, description, or top-level parameter names for BM25.
Avoid adding API vendor names as search hints. The hint text helps clients
formulate queries and is outside the ranked fields. Try the phrases with
`x402_treazury_tools_search` against the destination inventory and inspect the
first results for competing matches.

## Tool documentation

Keep agent-facing endpoint instructions, tool descriptions, and parameter
descriptions concise, direct, and actionable:

- **Include:** What the tool does, required and meaningful optional inputs,
  what the response contains, and constraints that affect the caller's next
  step (such as exact-amount requirements, single-use deposit addresses,
  partial-result semantics, or non-retry warnings).
- **Omit:** Internal Rust module names, TOML configuration keys, maintainer
  rationale, provider probe history, repetitive boilerplate, and tool IDs that
  depend on the registration name.
- **Do not discourage tool use:** Paid tools exist to be used from a pre-funded
  treasury. Do not add defensive disclaimers, generic domain truisms,
  operator-cap warnings, or pricing-uncertainty handwringing (such as
  "unknown does not mean free" or "not guaranteed quotes") that make agents
  hesitate to call paid tools or loop on failing free workarounds. State
  concrete behavioral constraints (such as non-retry warnings on failed paid
  calls) plainly without deterring normal use.
- **Field-specific edits:** Keep behavioral rules in `description` and
  supplemental billing terms in `pricing_description`. A parameter note edits
  its named schema description. The published tariff and each input's
  request contract have their own sources.
  [Description overrides](configuration.md#observed-prices-and-description-overrides)
- **Keep maintainer notes in docs:** Put compatibility rationale, probe limits,
  and upgrade caveats in `providers/CAVEATS.md` and the owning reference guide,
  not in MCP tool text.
