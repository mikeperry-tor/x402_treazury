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
max_price_usd = "1.00"
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
| Agent402 | `agent402.toml` | Web-focused default; alternate lowercase tags select reviewed catalog subsets |
| AgentFund | `agentfund.toml` | Vendor OpenAPI |
| AgentUtility | `agentutility/provider.toml` | 15 research operations + help; enriched cluster tags; managed payments omit extensions |
| Arkham | `arkham.toml` | Vendor OpenAPI |
| BlockRun | `blockrun.toml` | Six synchronous chat/search tools and lazy help; jobs/media/lifecycle excluded |
| Claw402 | `claw402.toml` | Twelve market overview reads and lazy help; exact curated allowlist |
| botsmith | `botsmith.toml` | Vendor OpenAPI |
| Brazilayer | `brazilayer.toml` | Vendor OpenAPI |
| Curl HTTP Request | `curl/provider.toml` | Local schema; POST `/curl` returns status, headers and body text; advertised $0.01 |
| Concordance | `concordance/provider.toml` | Local `concordance/openapi.json` |
| Deepline | `deepline.toml` | Vendor OpenAPI |
| Exa | `exa.toml` | Vendor OpenAPI; Search, Contents and lazy help only |
| Genuine Good Grants | `genuinegood.toml` | Vendor OpenAPI |
| Glassnode | `glassnode/provider.toml` | Local `glassnode/openapi.json` |
| Google Trends | `google-trends.toml` | Vendor OpenAPI |
| Kronos | `kronos.toml` | Vendor OpenAPI |
| Locus | `locus/provider.toml` | Local `locus/openapi.json` |
| LoneStarOracle | `lonestar.toml` | Vendor unified OpenAPI |
| OneShot | `oneshot.toml` | Synchronous web search and lazy help; async workflows excluded |
| Otto | `otto.toml` | Vendor OpenAPI |
| People Data Labs | `pdl.toml` | Vendor OpenAPI |
| RegimeShift | `regimeshift.toml` | Vendor OpenAPI |
| SocialFetch | `socialfetch.toml` | Vendor tagged OpenAPI |
| StableEnrich | `stableenrich.toml` | 32 synchronous JSON/image operations and lazy help; select upstreams by tag |
| Straits | `straits/provider.toml` | Local `straits/openapi.json` |
| x402-list | `x402-list.toml` | Curated API-directory reads |
| x402stock | `x402stock.toml` | Vendor OpenAPI |

Curl additionally pins the vendor-published OpenAPI response envelope for discovery
relay use; see [relay configuration](../docs/configuration.md#paid-discovery-relay).

The five authored local catalogs are OpenAPI 3.1 request definitions, explicitly
labeled as local rather than vendor-published. They preserve the curated routes,
tags, descriptions, parameters, request bodies and embedded prices from the
Python reference digests. Prices use the `x-payment-info` extension. Responses
have a generic description because their schemas are not modeled. Preserve the
Locus gateway body rewrite and job exclusions, Straits free/premium split,
Concordance POST pricing, and Glassnode's discovery workflow when editing them.

`tests/provider_catalogs.rs` pins 861 tool definitions across the 23 providers
with committed fixtures. `tests/config.rs` pins settings for all 27 providers.
Tests need no Python or live vendor access. See [provider caveats](CAVEATS.md)
for route exclusions, pricing semantics and vendor-specific workflows.

The x402-list provider exposes service search/detail, rankings, categories and networks.
Directory reads can become paid after a shared-IP free quota; the ordinary wallet cap
still applies. Listings do not guarantee compatible payment or supply executable API
schemas. Pricing probes and directory writes are excluded.

New provider defaults should select task-facing operations and omit health, status,
account, administration and other diagnostic endpoints unless essential to an
agent workflow. Use exact `include_operations` when only part of an API supports
x402; this also prevents future vendor routes from silently expanding the catalog.
Attach `help_url` when a single useful guide covers the selected toolset; help is
fetched only on demand and cached for the process lifetime. Preserve complete
selected-operation schemas and usage guidance instead of truncating them to save
context. See Exa in [the caveats](CAVEATS.md) for this pattern.

Agent402 has a much larger catalog than most providers. Its default `tags = ["web"]`
produces 48 API tools plus help in the fixture; `--tags data,crypto` replaces that
selection. Run `--provider providers/agent402.toml catalog tags` to inspect the vendor's
24 tags. Counts there describe the source before exclusions, not necessarily the
selected inventory. See [Agent402 caveats](CAVEATS.md#agent402toml--agent402tools)
for supported subsets, excluded workflows and examples.

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
dynamic tools and `treazury_tool_call`, retaining existing scope/revision checks.
See [artifact storage plan](../docs/plans/artifact_storage.md) for large files,
audio/video, retrieval and optional asset capture.

See the [provider caveats](CAVEATS.md) for media exclusions, deferred jobs, sampled payment extensions and help coverage.

Apollo via Locus was reviewed but has no enabled provider definition. Its
integration blocker and unsigned Tor results are documented in
[provider caveats](CAVEATS.md). A public x402 quote
is not sufficient evidence that a useful paid call is supported.
