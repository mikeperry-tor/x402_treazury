# Provider definitions

Rust reads TOML configuration. A deployment source can declare all settings
inline or extend one of these reusable files:

```toml
[sources.exa]
extends = "../../providers/exa.toml"

[servers.research]
listen = "127.0.0.1:8000"
bearer_token_env = "X402_TREAZURY_MCP_TOKEN"
wallet = "research"
sources = ["exa"]
```

This provider-binding excerpt belongs to the complete
[managed deployment](../examples/deployments/managed.toml), which defines the
research pool, treasury, funding and source-management settings. Provider files
describe APIs; deployments choose payment identities. Static wallets remain supported in the
[configuration reference](../docs/configuration.md).

The `extends` path is relative to the deployment file. A local `spec` path is
relative to the file declaring that value. Local fields replace entire inherited
values: lists do not concatenate, an empty list clears a selection, and a local
`overrides` table replaces the provider's entire override map. Providers cannot
extend other files. Unknown keys fail validation. All generic catalog fields
can be overridden at the source, including `spec`, `base_url`, `prefix`,
`include`, `exclude`, `tags`, `exclude_tags`, descriptions and HTTP policy.
Use listener tag/tool filters for multiple views of one shared source catalog.

`--provider providers/pdl.toml` runs a single provider. `--config` loads
sources, wallets and listeners. Add `config show` to either to inspect file
composition and per-field origins without fetching specs or reading secret
values. See the [configuration reference](../docs/configuration.md) for complete usage.

| Provider | Definition | Catalog |
| --- | --- | --- |
| Agent402 | `agent402.toml` | Full supported catalog; advisory web research recommendations and lowercase tags |
| AgData | `agdata.toml` | Web/social data; both x402-capable aliases; screenshot/PDF JSON |
| AgOSINT | `agosint.toml` | x402 registry gateway; root server URL repair |
| AGProxy | `agproxy.toml` | Complete pinned `agproxy-openapi.json`; residential fetch and proxy packs |
| Datakoot | `datakoot.toml` | Public-data gateway; apex-domain help |
| HireScrape | `hirescrape.toml` | Synchronous paid scraper namespace; SIWX lifecycle omitted |
| Horizon Pulse | `horizonpulse.toml` | Complete pinned `horizonpulse-openapi.json`; screenshot mapping and atomic-price caveat |
| NetIntel | `netintel.toml` | Canonical POST/body contracts and image data-URI mapping |
| TeleSINT | `telesint.toml` | Stateless Telegram intelligence; watch ownership unsupported |
| RelayShield phone | `relayshield/phone-reputation.toml` | Authored vendor-omitted request contract |
| AgentFund | `agentfund.toml` | Vendor OpenAPI; no additional compatibility restriction identified |
| AgentUtility | `agentutility/provider.toml` | Full supported aliases/catalog; advisory research recommendations; enriched provenance |
| Arkham | `arkham.toml` | Vendor OpenAPI |
| AX1 Verdict | `ax1/provider.toml` | External request projection; token Verdict submission and free public receipt |
| BlockRun | `blockrun.toml` | Synchronous catalog with fixed-symbol reads; async jobs/ownership excluded |
| Claw402 | `claw402.toml` | Full catalog, advisory market reads, sparse RootData JSON bodies |
| CoinMarketCap | `coinmarketcap/provider.toml` | Authored union of all public keyless routes and four x402 counterparts |
| botsmith | `botsmith.toml` | Vendor OpenAPI; no additional compatibility restriction identified |
| Brazilayer | `brazilayer.toml` | Vendor OpenAPI; no additional compatibility restriction identified |
| Curl HTTP Request | `curl/provider.toml` | Local schema; POST `/curl` returns status, headers and body text; advertised $0.01 |
| Concordance | `concordance/provider.toml` | Local `concordance/openapi.json` |
| Deepline | `deepline.toml` | Vendor OpenAPI; no additional compatibility restriction identified |
| Degenscan Intel | `degenscan.toml` | Event intelligence, keyless Carry reads and public forecast submit/poll |
| Exa | `exa.toml` | Vendor OpenAPI; Search, Contents and lazy help only |
| Genuine Good Grants | `genuinegood.toml` | POST grant research; search/detail recommendations; bearer pass excluded |
| Glassnode | `glassnode/provider.toml` | Local `glassnode/openapi.json` |
| Google Trends | `google-trends.toml` | Vendor OpenAPI |
| IPIntel.ai | `ipintel.toml` | Domain/DNS/IP, satellite and developer tools; core lookup recommendations |
| Kronos | `kronos.toml` | Market data plus public status/methodology; existing names preserved |
| Locus | `locus/provider.toml` | Local `locus/openapi.json` |
| LoneStarOracle | `lonestar.toml` | Full supported gateway, including public previews/feeds; research recommendations |
| Market Intelligence API | `marketintelligence.toml` | Market research and analytics; POST opportunity scans; prepaid/webhook workflows excluded |
| Nansen | `nansen.toml` | Keyless paid analytics; API-key account/trading and SSE agents excluded |
| News Gurus | `newsgurus/provider.toml` | Vendor x402 catalog projection, including agent-status; lazy help |
| OneShot | `oneshot.toml` | Synchronous search and public discovery; authenticated jobs excluded |
| Otto | `otto.toml` | Vendor OpenAPI |
| People Data Labs | `pdl.toml` | Vendor OpenAPI; no additional compatibility restriction identified |
| RelayShield | `relayshield.toml` | Vendor OpenAPI; optional authored `relayshield/phone-reputation.toml` companion |
| RegimeShift | `regimeshift.toml` | Vendor OpenAPI |
| SocialFetch | `socialfetch.toml` | Vendor tagged OpenAPI |
| StableEnrich | `stableenrich.toml` | Synchronous JSON/image catalog; specific lifecycle paths excluded |
| StableTravel | `stabletravel/provider.toml` | Full flight-data catalog with authored path-parameter repairs; cash fares, cached awards and operational reads |
| Straits | `straits/provider.toml` | Authored free/premium JSON, CSV, RSS and HTML contracts |
| Vaaya | `vaaya.toml` | Supported x402 JSON/image catalog; specific MPP/lifecycle boundaries |
| VoIPStore | `voipstore.toml` | Virtual-number catalog, quotes, purchases and SMS status; prepaid top-ups excluded |
| x402-list | `x402-list.toml` | Public directory reads, assessments and suggestions |
| x402stock | `x402stock.toml` | Full supported market/economic catalog; compact research recommendations |

Curl, AGProxy and Horizon Pulse pin complete vendor OpenAPI documents as runtime
assets. This lets relay bootstrap validate operations locally without buying or
fetching another catalog. Their reusable catalogs remain pinned until a deliberate
update; see [capture provenance and update instructions](CAVEATS.md#pinned-relay-contracts)
and [relay configuration](../docs/configuration.md#relay-fallback).

Authored local catalogs are explicitly labeled rather than presented as vendor
OpenAPI documents. Preserve complete documented request contracts, source
provenance and intentional enrichment, including `x-payment-info` price hints.
Generic response placeholders are not authoritative schemas; captured public
discovery objects retain their original documentation. The Locus gateway body
rewrite and job exclusions, Straits free/premium resources, Concordance pricing
and Glassnode metadata-to-metric workflow each have concrete consumers.

`tests/provider_catalogs.rs` checks concrete provider contracts and
`tests/config.rs` validates all definitions offline without settings snapshots.
Tests need no Python or live vendor access. See [provider caveats](CAVEATS.md)
for route exclusions, pricing semantics and vendor-specific workflows.

The x402-list provider exposes public directory reads, assessments and suggestions.
Directory reads can become paid after a shared-IP free quota; the ordinary wallet cap
still applies. Listings do not guarantee compatible payment or supply executable API
schemas. Owner-token workflows and an unrepresentable submit
schema are excluded. Embedded source management deliberately keeps three reads.

Optional advertised provider help may explain prices and request contracts before
a paid call. Unknown prices mean missing pricing information, not free calls.
Importing a schema and reading help do not establish paid-delivery qualification.

## Bundled provider metadata

The executable embeds the shipped definitions and referenced local OpenAPI assets.
`src/providers/assets.rs` is the membership manifest: stable IDs are normally the
TOML stem, a directory's `provider.toml` uses its directory name, and the phone
companion is `relayshield-phone-reputation`. Add new shipped definitions there;
keep settings and descriptions in their TOML, without copying them into the
manifest. Local schemas load independently of the installation directory.
No separate data installation, filesystem scan or writable index is required.
Private local overrides are neither bundled nor compilation identity inputs.

Providers selected in a relay method list declare `relay_format = "curl"`,
`"agproxy"` or `"horizonpulse"` once in their TOML. This selects the documented
request/response envelope; provider names, hosts and schema shapes do not infer it.
The field is optional for ordinary API providers. Listed providers require a
supported format and compatible operation/filter selection before paid dispatch.
Operator-authored `[relay_fallback.relays]` GET, HEAD and POST
lists determine selection and order; an adapter still checks whether it can
preserve the actual request. This metadata does not enter agent tool descriptions
or input schemas.

With `source_management = true`, `x402_treazury_providers_list` enumerates this
catalog, and `sources_search(local_only=true)` filters it without vendor requests
or wallet work. Both return selectable `provider_id` values for `source_add`;
`source_details` resolves exact declared API bases into the same candidates.
Registering by ID uses the reviewed configuration and embedded or remote schema.
A unique exact current remote specification URL also resolves that definition,
while existing generic URL registrations retain their accepted behavior. Unknown
or ambiguous selectors require correction rather than arbitrary selection.

Every shipped definition has a concise root `description` of supported
capabilities for offline search. This metadata is separate from operation
descriptions and `instructions_text`; external definitions may omit it. API-base
lookup uses exact normalized HTTPS origin and base path, retaining ambiguous
companions. A remote specification matches its complete URL. Definitions without
an explicit `base_url` have no API-base lookup key; a schema URL is not an API
identity. Bundled metadata does not establish registration, availability, funding
or live qualification.

## Provider policy

Reusable definitions should expose supported functionality, including newly added
supported routes. Restrict catalogs for concrete authentication/payment, request,
output/lifecycle or financial-authority incompatibilities. Diagnostic endpoints,
compactness, price and overlap with another provider are editorial choices, not
compatibility failures. Keep naming anchors when they determine generated names. Add an empty root
fallback when paths outside an anchor were hidden only for convenience; retain a
namespace restriction only for an identified support boundary such as x402 versus
MPP authentication. Recommendations cannot restore routes hidden by path filters.
Exact operation allowlists remain appropriate for verified payable islands in a
mixed API; explain why path or tag exclusions cannot describe that boundary.

Recommendations belong in explicit deployment selections, separate from reusable
compatibility defaults. Default selection collapses same-path GET/POST twins to POST before naming.
Provider/source `recommended_tools` carries advisory generated-name selectors;
`use_recommended_tools = true` explicitly selects that advice and intersects ordinary
filters. Omission leaves the compatible catalog visible. Include help deliberately,
replace advice when overriding the source prefix, and omit advice when the entire
catalog is the appropriate choice. `catalog tools --all-tools` inspects without
advice selection while retaining ordinary filters. See the
[selection semantics](../docs/configuration.md) for empty/stale advice and offline
metadata inspection.

Review method pairs using actual request, result and billing contracts. Uncertain
or meaningfully different variants need a documented exception; similar names or
prices alone do not establish equivalence. A catalog selection grants no payment
retry authority; see the [unsigned request contract](../docs/architecture.md#unsigned-x402-requests-and-post-retries).

Attach `help_url` when a useful guide covers the toolset; help is fetched on demand
and cached for the process lifetime. Preserve complete request schemas and usage
guidance. Keep `instructions_text` brief and limited to cross-tool workflows or
shared interpretation rules. Put operation requirements, sparse-schema arguments,
pagination quirks and coverage limitations in the affected tool descriptions or
parameter notes. Avoid capability inventories already present in tool metadata,
manual payment/sign/retry steps, client setup, marketing and maintainer provenance
in agent guidance; keep provenance and compatibility rationale in this guide and
the provider caveats. Preserve operation prices and billing distinctions rather
than repeating the server's general pricing caveat. Provider-specific tests and fixtures need a concrete quirk, historical
failure or authored transformation. Prefer small faithful inputs and assertions
for that responsibility, with synthetic shared tests for generic behavior. A new
provider requires neither a complete golden/settings snapshot nor paid
qualification or an artificial qualification-exclusion record. Runtime local
catalogs and payment evidence retain their own consumers and cannot be discarded
as redundant test data. See [testing policy](../docs/testing.md#provider-regressions)
and [fixture maintenance](../tests/fixtures/catalogs/README.md).

Agent402 exposes a broad supported catalog with advisory web-research
recommendations. Use source tags or listener selection to narrow it deliberately.
`catalog tags` reports the vendor inventory before exclusions. See
[Agent402 caveats](CAVEATS.md#agent402toml--agent402tools) for actual support
boundaries, supported media modes and request guidance.

## Inline image results

Successful PNG, JPEG and WebP HTTP bodies become MCP image content blocks. JSON
and text remain text unless an operator supplies a response mapping. Unsupported
binary MIME types, invalid UTF-8, invalid base64 and mismatched image signatures
return explicit errors; a paid call is never retried to repair output. Signature
checks identify file formats, not full image validity: there is no decoder,
transcoder or image-pixel inspection in Treazury. Clients must support the image
format and apply their own decoding limits.

Map a JSON envelope by exact HTTP method and original path, independently of the
source alias/tool prefix:

```toml
[response_mappings."POST /images"]
images = [
  { pointer = "/data/0/b64_json", mime_pointer = "/data/0/media_type" },
]

[response_mappings."POST /crop"]
images = [{ pointer = "/dataUri", encoding = "data_uri" }]

[response_mappings."POST /generate"]
images = [{ pointer = "/image", mime_type = "image/png" }]

[image_limits]
max_image_bytes = 4194304
max_total_bytes = 8388608
max_images = 4
```

These are the default decoded-byte/count limits. They apply per call across all
attachments, in addition to `max_response_bytes` (default 16 MiB) for the complete
HTTP body, including base64 overhead. Encoding is `base64` by default; data URIs
must explicitly use `;base64`. JSON pointers are exact, non-root pointers with
standard `~0`/`~1` escaping, without wildcards. Mapped fields are required strings.
Use exactly one of fixed `mime_type` or `mime_pointer` for plain base64; a data URI
can supply its own MIME. Supported types are `image/png`, `image/jpeg`, `image/webp`.
No MIME guessing, array discovery, optional-field fallback or URL fetching occurs.

Each extracted JSON field becomes an explicit `{treazury_attachment, mime_type}`
marker in the metadata text, indexed from 1 into the following image blocks.
Other JSON fields are retained. All extractions must succeed: no partial successful
result is returned. Text truncation limits apply only to the text block and retain
the existing visible marker; encoded image data is never truncated. Inline data is
not persisted and there is no artifact retrieval URI.

Mapping and limit tables replace inherited tables in full, like other source
fields. Provider response mappings also appear in `config show` and resolved tool
inventories. Limits and unused mappings validate at config load. Agent-added sources
use the default image limits; agents cannot supply provider response-mapping TOML
through source registration. Raw images use the same typed path for static tools,
dynamic tools and `x402_treazury_tool_call`, retaining existing scope/revision checks.
See [artifact storage plan](../docs/plans/artifact_storage.md) for large files,
audio/video, retrieval and optional asset capture.

See the [provider caveats](CAVEATS.md) for media exclusions, deferred jobs, sampled payment extensions and help coverage.

Apollo via Locus was reviewed but has no enabled provider definition. Its
request-ID retry/recovery uncertainty is documented in
[provider caveats](CAVEATS.md). A public x402 quote
is not sufficient evidence that a useful paid call is supported.

Source management automatically supplies x402 List browsing, ranked recommendations
and service details using the embedded `x402-list/directory.openapi.json` contract. The `x402-list.toml` provider is only
needed when exposing the broader directory API separately. The embedded three-operation
contract is checked against the pinned directory fixture by the discovery tests.
