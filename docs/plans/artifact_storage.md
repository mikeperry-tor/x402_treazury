# Artifact storage and retrieval

## Objective and existing behavior

Extend Treazury's tool results with bounded, authenticated artifacts for PDFs,
audio, video, large images and other approved files. Agents should receive a useful
summary and retrieval reference without putting large base64 strings into model
context. Preserve payment uncertainty, caller permissions and Tor isolation.

`src/output.rs` currently preserves HTTP bytes/MIME, returns small PNG/JPEG/WebP
responses as MCP image blocks, and extracts explicitly mapped JSON base64/data-URI
fields. Metadata contains numbered attachment markers. `catalog::Config` has
`image_limits` and method/path keyed `response_mappings`; `ToolSpec` carries the
resolved mapping. `PaidClient::execute_response` reports whether payment was
submitted. `BoundTool::invoke_output` and `Server::invoke_output` share the typed
path for direct calls and `treazury_tool_call`. Text-only convenience methods
reject image results. There is no artifact store, resource retrieval capability,
automatic asset download, transcoding or resumable job implementation.

The separate deferred polling plan governs asynchronous provider jobs. Returning
a file from a completed request must not implicitly enable job submission/polling.

## User-facing contract

Artifact storage is opt-in. Disabled mode retains current explicit errors when
output cannot be delivered inline. Configure an owner-only directory, retention,
per-artifact bytes, aggregate storage bytes, artifact count, concurrent writers,
and retrieval limits. Inspect configuration without creating/opening the directory.
Keep stdio and multi-listener HTTP on the same storage implementation.

A successful tool call returns:

- Short metadata: artifact ID, safe display name, MIME, exact byte size, digest,
  expiration, and whether it was captured or derived.
- A resource link using a Treazury URI, plus remaining vendor JSON metadata.
- An explicit statement identifying extracted payload fields. Never describe a
  placeholder as the original vendor value or silently omit additional files.

Do not return a local filesystem path as the remote retrieval mechanism. Initial
retrieval should use authenticated MCP `resources/read` with a bounded base64 blob
response. Large artifacts that exceed this retrieval limit need an explicitly
configured authenticated streaming HTTP download endpoint; implement that before
advertising large-video support. Resource links alone do not guarantee that a
particular client can display, download or forward a file. Qualify supported clients
and document alternatives. Do not inline blobs in resource listings.
Streaming downloads use safe attachment filenames, `nosniff`, and private/no-store
caching; content must never become executable same-origin HTML/SVG. Authenticate
every request rather than placing bearer credentials in download URLs.

Expose only artifacts accessible to the current authenticated listener/principal.
Provide bounded listing/metadata/delete tools only if useful, separately granted
from source mutation tools; identifiers and URLs must never be implicit grants.
Do not automatically export files into client workspaces.

## Ownership and authorization

Use the existing bearer-authenticated listener identity as the principal, including
fallback tool calls. Wallet identity is provenance, not authorization: listeners
sharing a payer must not inherit access to each other's outputs. Sharing a source
or process-wide catalog entry likewise does not share results. Stdio gets an
explicit local principal. Artifact IDs are opaque, random and non-enumerable, but
authorization still checks every retrieval and deletion.

Capture principal, source ID/revision, tool identity and actual executing payment/
network identity when the request starts. Store only necessary provenance; do not
retain private keys, payment signatures, bearer tokens or signed asset URLs in
ordinary metadata/logs. Artifact ownership persists independently of catalog
changes. Removing a source does not silently erase files or grant a new source
access; listener removal/revocation denies retrieval while normal retention handles
cleanup. Document behavior when a listener name is reused across restarts; bind
persistent ownership to an installation/listener identity instead of trusting a
recycled display name.

## Storage lifecycle and bounded concurrency

Use a dedicated storage module/actor with a bounded command queue. Reserve aggregate
bytes and count before accepting writes. Streaming writers charge bytes incrementally
under one shared budget; concurrent calls across all servers cannot each consume the
full free quota. Never await disk I/O under the catalog lock or payer admission gate.

Create private temporary files, stream bounded content, validate format/digest,
flush/sync, then atomically publish the completed file and metadata. Only completed
artifacts are retrievable. Define recovery ordering between file publication and
metadata commit, and clean orphan temporary/completed files after crashes. Reject
symlinks, hard-link aliases and unsafe path traversal according to the repository's
filesystem rules. Never use vendor filenames as storage paths. Exclude storage
from git and prevent aliases with treasury, registry, config or key files.

Retention uses creation/expiry timestamps and an operator-selected persistent or
process lifetime. Readers hold leases while streaming; garbage collection and
explicit deletion cannot race readers into partial successful downloads. Bound
reader concurrency and lifetime. Full disk, quota exhaustion, cancellation and
shutdown must leave either a completed artifact or a recoverable incomplete state,
never a successful dangling reference. Do not delete unexpired files silently to
make room; report rejection unless an explicit eviction policy defines observable
notifications and retrieval errors.

At-rest encryption is a separate deployment decision: require a documented choice
before implementing persistent storage. Treasury keys must not become artifact
keys implicitly. Private directory permissions alone do not provide encryption.
Specify key lifecycle/backup and deletion guarantees if encryption is selected.

## HTTP response and mapping integration

Refactor bounded response collection so configured artifact outputs can stream to
storage without first buffering their entire body. Keep inline images bounded as
today. Unknown binary types remain errors unless allowed by operator/provider
policy; never infer executable rendering permissions from Content-Type alone.

Extend `response_mappings` with explicit output kind, fixed/pointer MIME, optional
fields, and bounded array extraction where needed. Preserve JSON metadata with
artifact markers, reject ambiguous mappings, and test changes to envelope schemas.
Base64 decoding must be incremental or bounded before allocation; encoded, decoded,
aggregate and attachment-count limits apply independently. Fail the result atomically
if any required extraction fails. Optional absence is different from invalid data.
Do not parse every string heuristically or silently replace existing text contracts.

## Provider requirements and activation boundaries

Existing supported synchronous image routes remain enabled. Other candidates
require storage, optional extraction, URL capture or polling; actual paid response
contracts still need qualification.

| Provider/routes | Output contract | Required work |
| --- | --- | --- |
| BlockRun image/edit/video jobs | Guide describes async companion GETs and generated media | Resolve polling/ownership and actual URL/base64 response branches before enabling submission; bounded identity-preserving capture. |
| BlockRun speech/sound effects/music | Binary or generated audio, depending on endpoint/model | Qualify MIME/codec and sync versus job behavior; authenticated artifact retrieval rather than text decoding. |
| Claw402 image generation | OpenAPI advertises image generation but response schema is only a generic object | Capture and review the actual response contract before choosing a base64 pointer or URL mapping; do not guess. Small supported images may need only existing inline mapping. |
| Agent402 PDF merge/extract-pages/rotate and images-to-PDF | JSON `pdfBase64`, `pages`, `bytes` | Explicit PDF artifact mapping; validate actual decoded length and retain page metadata. |
| Agent402 audio-convert/audio-normalize and speech | MP3 base64 or audio bodies depending on operation | Per-operation codec/MIME mapping, bounded duration/bytes, authenticated download; not image blocks. |
| Agent402 `/v1/videos/generations` | Guide documents inline base64 MP4 for one four-second clip | Verify exact JSON pointer/MIME/size and implement streaming retrieval; slow upstream generation alone is not a client job contract. |
| Agent402 image resize/convert/thumbnail | Binary PNG/JPEG/BMP selected by `format` | BMP artifact support, or a separately enforced PNG/JPEG input restriction before payment. Routing does not generally validate submitted values against JSON Schema; enforce the restriction before payment, not only in descriptions/schema hints. |
| Agent402 favicon-grab | `found:false`, URL-only above 256 KB, or `/dataUri` with `/contentType`, including ICO/SVG | Optional extraction with explicit format branches; preserve absence and URL-only results; no automatic URL fetch. |
| AgentUtility QR-code-generate | `/data_base64`, `/content_type`, `format` PNG or SVG | PNG inline versus SVG artifact, or an enforced PNG-only variant. Never render SVG as active same-origin content. |
| AgentUtility browser-session | HTML metadata or optional base64 PNG screenshot | Qualify screenshot field layout; current success example documents only HTML. Do not guess a pointer or blanket-decode strings. |
| AgentUtility ipfs-fetch | Arbitrary CID content as base64 (default), text or JSON; `content_type`, `size_bytes`, gateway metadata; upstream 5 MB cap | Determine encoded-content field/branch, use approved MIME policy, preserve JSON/text, bound decoded bytes and avoid duplicate alias tools. Provider cap does not replace local limits. |
| AgentUtility hosted image/audio/video/document conversion | `image_url`, `audio_url`, `video_url`, `pdf_url` or route-specific URL fields | Already exposed as JSON metadata; later explicit URL capture, with per-route mapping, expiry and original identity. No inferred polling. |
| Locus satellite change | Optional `/imagery/before/dataBase64` and `/imagery/after/dataBase64`, MIME `/imagery/contentType` | Optional inline PNG mappings; metadata-only default and charge-free diagnostic responses must remain valid. This small enhancement can precede full artifact storage. |
| Locus property-flyer | Expiring `imageUrl`, PNG dimensions, provenance/rights fields | Optional URL capture and retention; preserve evidence, attribution and expiration rather than returning only pixels. |
| Locus property-update job | Independently ready report/PDF/video URLs; `flyerHandoff` with `pdfUrl`/`expiresAt` | Polling integration plus per-deliverable capture. Private job token and public/share proof are different capabilities. Do not manufacture URLs/tokens or wait for video to expose a ready PDF. |
| StableEnrich Solar data-layers / rgb-image | Scientific GeoTIFF URLs or image URL/binary branches | Optional URL capture; preserve original scientific files and geospatial metadata. PNG/JPEG direct bodies are already supported. |
| StableEnrich Aerial View | Render/lookup job yielding video URLs; lookup can be paid | Both polling budget/identity policy and later artifact capture; storage alone must not enable submission. |
| Exa batches | Short-lived presigned `resultsUrl` for JSONL | Credential-based async integration, safe status-based URL refresh, streamed text artifacts and bounded previews. API auth must not reach the object-store host. |
| Straits bulk export | Synchronous JSON or CSV history archive, already exposed | Optional text artifacts with bounded previews for large histories; no job polling needed. Preserve CSV as data rather than rendering active content. |
| Otto image/video generation | Loose `status`/`data` response schema | Existing JSON tools remain; qualify actual output fields before mapping bytes or URLs. Do not infer a poll contract from fal.ai usage. |

Sources: [Agent402 spec](https://agent402.tools/openapi.json),
[AgentUtility spec](https://x402.agentutility.ai/openapi.json),
[Locus spec](https://api.locus.report/openapi.json),
[StableEnrich guide](https://stableenrich.dev/llms.txt),
[Exa spec](https://api.exa.ai/openapi.json),
[Otto spec](https://x402.ottoai.services/openapi.json).

Optional extraction must distinguish absent fields in a valid metadata/diagnostic
branch from malformed present payloads. For image pairs, define whether both must
be present together; never silently deliver one side of a before/after comparison.
Use reviewed branch/field rules, not arbitrary provider expressions. Source policy,
license and attribution fields survive extraction. Locus also advertises cache/
redistribution policies; do not promise durable retention for a `no_store` result.
Define a visible policy conflict rather than silently dropping or retaining it.

Large JSONL and text files are artifacts too. Provide bounded, explicitly marked
previews without pretending they are complete results. Treat URL expiration, job
expiration and local artifact expiration as distinct states. A reviewed provider
may support refreshing a result URL via status lookup (Exa does); that must not
replay the original paid operation or enable automatic paid lookup by default.

Initial storage performs no transcoding. Any later converter needs bounded image
pixels, audio duration, process time/memory, codec isolation and separate provenance.

## Optional asset URL capture and network policy

URL capture is a later, separately enabled capability, not an automatic consequence
of seeing a URL in JSON. Use explicit mapping or a scoped capture request tied to
the original result. Apply public-destination validation, response limits, deadline,
redirect policy and allowed schemes. Never forward vendor authorization/payment
headers to an asset host, and never pay a second challenge automatically.

All capture traffic uses `src/network.rs`. Wallet-linked assets retain the actual
original EVM isolation identity even if rotation occurs before retrieval; catalog
identity is not a substitute. Persist this binding if deferred capture survives a
restart. Do not expose derived SOCKS credentials. A client fetching an external
URL itself is outside Treazury's Tor policy; make that boundary clear. Signed URLs
are sensitive and may expire: report failure, never repeat the original paid
operation just to obtain a replacement link.

## Payment and failure semantics

Artifact handling does not change admission, signing, settlement or replay rules.
A download, decode, storage or quota failure after signing can occur after settlement;
return an explicit potentially-paid error and never retry the original request.
Preflight known storage limits before signing where possible, without claiming that
preflight guarantees a response will fit. Keep incomplete artifact cleanup separate
from payment/exposure reconciliation. Ordinary resource retrieval reads stored bytes
and must not contact the vendor, spend money or rotate a wallet.

Logs and agent-visible errors identify the affected resource, configured limit and
setting without payloads, sensitive URLs or credentials. Report expiration, revoked
access, failed capture and incomplete storage distinctly. Attachment extraction is
observable. No silent truncation, dropped files or pretend-success resource links.

## Implementation milestones (one commit each)

1. **Configuration and ownership contract.** Resolve persistent principal identity,
   at-rest protection, retrieval interface and retention decisions. Add strict TOML,
   inspection output and capability/permission checks. Document operational limits.
2. **Bounded store.** Implement reservations, private streaming writes, atomic
   publication, metadata, reader leases, expiry and crash recovery. Keep it independent
   of paid execution until fault-injection tests pass.
3. **Typed artifact outputs.** Integrate captured response bytes and mapped base64
   envelopes; retain metadata and payment uncertainty. Enable small fixture-only
   artifacts initially and preserve existing text/inline image contracts.
4. **Authenticated retrieval.** Add MCP resources and, if needed for the selected
   release formats, streaming downloads. Test caller scope and resource-link behavior
   over stdio and authenticated Streamable HTTP. Add explicit deletion if authorized.
5. **Provider qualification.** Review actual response contracts and add curated
   routes, fixtures and documentation for supported formats. Test real clients using
   synthetic files first. Live paid tests require explicit user funding authorization.
6. **Optional URL capture.** Implement only after identity persistence, SSRF/redirect
   handling, Tor tests and expiring-URL semantics are complete. Keep polling separate.
   Add reviewed status-based link refresh only for providers with an explicit contract;
   prevent duplicate capture of concurrent ready-artifact notifications.

## Validation and acceptance

Test exact bounds and one-over for bytes/count/concurrency; malformed base64,
unexpected MIME, incomplete streams, dishonest Content-Length and conflicting field
maps; absent optional fields versus invalid present fields, diagnostic-only results,
partially missing image pairs, PNG/SVG branches, URL-only favicon results, JSONL
preview boundaries, and source retention-policy conflicts; metadata preservation and complete absence of raw base64 in text summaries.
Exercise simultaneous calls across servers/wallets, quota reservation rollback,
reader/expiry/delete races, accepted-work cancellation, full disk and crash/restart
at every publication boundary. Verify no partial file is ever served as complete.

Test cross-listener denied retrieval with shared wallets/sources, source removal,
listener revocation/reuse, guessed IDs, path traversal, symlink/hard-link refusal,
and credential-free config inspection. Verify payment signs/submits once despite
storage failure and retrieval makes zero vendor calls. Test direct and fallback MCP
tools identically. For URL capture, prove original-wallet SOCKS identity after rotation,
remote DNS, no direct fallback and no authentication forwarding across origins.

Run targeted suites in both feature modes sequentially, default all-target regression,
Clippy, formatting and network-audit checks. Refresh provider goldens after reviewing
complete contracts. A release requires demonstrated retrieval in supported clients,
documented disk/retention behavior, and explicit errors/logs for every enforced bound.
