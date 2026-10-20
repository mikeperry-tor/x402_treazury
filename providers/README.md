# Provider definitions

Rust reads TOML configuration. A deployment source can declare all settings
inline or extend one of these reusable files:

```toml
[sources.socialfetch]
extends = "../providers/socialfetch.toml"
timeout = 90

[servers.research]
listen = "127.0.0.1:8000"
bearer_token_env = "RESEARCH_MCP_TOKEN"
wallet = "default"
sources = ["socialfetch"]
tags = ["Twitter", "YouTube"]

[wallets.default]
mode = "static"
private_key_env = "EVM_PRIVATE_KEY"
max_api_payment_usdc = "1.00"
```

The `extends` path is relative to the deployment file. A local `spec` path is
relative to the file declaring that value. Local fields replace entire inherited
values: lists do not concatenate, an empty list clears a selection, and a local
`overrides` table replaces the provider's entire override map. Providers cannot
extend other files. Unknown keys fail validation. All generic catalog fields
can be overridden at the source, including `spec`, `base_url`, `prefix`,
`include`, `exclude`, `tags`, `exclude_tags`, descriptions and probe settings.
Use listener tag/tool filters for multiple views of one shared source catalog.

`--provider providers/pdl.toml` runs a single provider. `--config` loads
sources, wallets and listeners. Add `config show` to either to inspect file
composition and per-field origins without fetching specs or reading secret
values. See the [configuration reference](../docs/configuration.md) for complete usage.

| Provider | Definition | Catalog |
| --- | --- | --- |
| Agent402 | `agent402.toml` | Full supported catalog; advisory web research recommendations and lowercase tags |
| AgOSINT | `agosint.toml` | x402 registry gateway; root server URL repair |
| Datakoot | `datakoot.toml` | Public-data gateway; apex-domain help |
| HireScrape | `hirescrape.toml` | Synchronous paid scraper namespace; SIWX lifecycle omitted |
| NetIntel | `netintel.toml` | Canonical POST/body contracts and image data-URI mapping |
| TeleSINT | `telesint.toml` | Stateless Telegram intelligence; watch ownership unsupported |
| RelayShield phone | `relayshield/phone-reputation.toml` | Authored vendor-omitted request contract |
| AgentFund | `agentfund.toml` | Vendor OpenAPI |
| AgentUtility | `agentutility/provider.toml` | Full supported aliases/catalog; advisory research recommendations; enriched provenance |
| Arkham | `arkham.toml` | Vendor OpenAPI |
| BlockRun | `blockrun.toml` | Synchronous catalog with fixed-symbol reads; async jobs/ownership excluded |
| Claw402 | `claw402.toml` | Full catalog, advisory market reads, sparse RootData JSON bodies |
| botsmith | `botsmith.toml` | Vendor OpenAPI |
| Brazilayer | `brazilayer.toml` | Vendor OpenAPI |
| Curl HTTP Request | `curl/provider.toml` | Local schema; POST `/curl` returns status, headers and body text; advertised $0.01 |
| Concordance | `concordance/provider.toml` | Local `concordance/openapi.json` |
| Deepline | `deepline.toml` | Vendor OpenAPI |
| Exa | `exa.toml` | Vendor OpenAPI; Search, Contents and lazy help only |
| Genuine Good Grants | `genuinegood.toml` | POST grant research; search/detail recommendations; bearer pass excluded |
| Glassnode | `glassnode/provider.toml` | Local `glassnode/openapi.json` |
| Google Trends | `google-trends.toml` | Vendor OpenAPI |
| IPIntel.ai | `ipintel.toml` | Domain/DNS/IP, satellite and developer tools; core lookup recommendations |
| Kronos | `kronos.toml` | Market data plus public status/methodology; existing names preserved |
| Locus | `locus/provider.toml` | Local `locus/openapi.json` |
| LoneStarOracle | `lonestar.toml` | Full supported gateway, including public previews/feeds; research recommendations |
| News Gurus | `newsgurus/provider.toml` | Local projection of vendor x402 catalog; 48 paid reads plus lazy help |
| OneShot | `oneshot.toml` | Synchronous search and public discovery; authenticated jobs excluded |
| Otto | `otto.toml` | Vendor OpenAPI |
| People Data Labs | `pdl.toml` | Vendor OpenAPI |
| RelayShield | `relayshield.toml` | Vendor OpenAPI; optional authored `relayshield/phone-reputation.toml` companion |
| RegimeShift | `regimeshift.toml` | Vendor OpenAPI |
| SocialFetch | `socialfetch.toml` | Vendor tagged OpenAPI |
| StableEnrich | `stableenrich.toml` | Synchronous JSON/image catalog; specific lifecycle paths excluded |
| Straits | `straits/provider.toml` | Authored free/premium JSON, CSV, RSS and HTML contracts |
| Vaaya | `vaaya.toml` | Supported x402 JSON/image catalog; specific MPP/lifecycle boundaries |
| x402-list | `x402-list.toml` | Public directory reads, assessments and suggestions |
| x402stock | `x402stock.toml` | Full supported market/economic catalog; compact research recommendations |

Curl additionally pins the vendor-published OpenAPI response envelope for discovery
relay use; see [relay configuration](../docs/configuration.md#paid-discovery-relay).

The five authored local catalogs are OpenAPI 3.1 request definitions, explicitly
labeled as local rather than vendor-published. They preserve the supported routes,
tags, descriptions, parameters, request bodies and embedded prices from the
Python reference digests. Prices use the `x-payment-info` extension. Responses
have a generic description because their schemas are not modeled. Preserve the
Locus gateway body rewrite and job exclusions, Straits free/premium split,
Concordance POST pricing, and Glassnode's discovery workflow when editing them.

`tests/provider_catalogs.rs` checks concrete provider contracts and
`tests/config.rs` validates all definitions offline without settings snapshots.
Tests need no Python or live vendor access. See [provider caveats](CAVEATS.md)
for route exclusions, pricing semantics and vendor-specific workflows.

The x402-list provider exposes public directory reads, assessments and suggestions.
Directory reads can become paid after a shared-IP free quota; the ordinary wallet cap
still applies. Listings do not guarantee compatible payment or supply executable API
schemas. Pricing probes, owner-token workflows and an unrepresentable submit
schema are excluded. Embedded source management deliberately keeps three reads.

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
metadata inspection. Until provider normalization,
existing provider selections below describe current behavior, including editorial
subsets scheduled for normalization. See the
[implementation plan](../docs/plans/provider_curation.md).

Review method pairs using actual request, result and billing contracts. Uncertain
or meaningfully different variants need a documented exception; similar names or
prices alone do not establish equivalence. A catalog selection grants no payment
retry authority; see the [unsigned request contract](../docs/architecture.md#unsigned-x402-requests-and-post-retries).

Attach `help_url` when a useful guide covers the toolset; help is fetched on demand
and cached for the process lifetime. Preserve complete request schemas and usage
guidance. Provider-specific tests and fixtures need a concrete quirk, historical
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
integration blocker and unsigned Tor results are documented in
[provider caveats](CAVEATS.md). A public x402 quote
is not sufficient evidence that a useful paid call is supported.

Source management automatically supplies x402 List browsing, ranked recommendations
and service details using the embedded `x402-list/directory.openapi.json` contract. The `x402-list.toml` provider is only
needed when exposing the broader directory API separately. The embedded three-operation
contract is checked against the pinned directory fixture by the discovery tests.
