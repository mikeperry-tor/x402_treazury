# Reviewed provider sweep preset

`providers.example.toml` defines the 23-provider baseline sweep, independently
pinned by `../fixtures/provider_requests.json`: 63 explicit cases consisting of
22 paid requests ($0.20 reservation each, $4.40 running API authorization
budget), 1 unsigned x402-list directory read, and 40 unsigned help fetch and
cache-hit checks across 20 help tools (OneShot is excluded).

- **GenuineGood method:** Uses canonical `genuinegood_search` (`POST` JSON
  under default `prefer_post = true`), validated offline against captured
  request contracts. Earlier funded `GET` evidence does not qualify the `POST`
  request.
- **Template setup:** Copy `tests/live/deployments/live-tor-providers.toml` and
  `providers.example.toml` to private paths, fill in all placeholders
  (treasury ID, run ID, absolute paths, Tor binary, and finite UTC windows),
  and keep all declared wallet pools configured (`auto_fund = false`, zero new
  funding jobs, paid cases targeting `coverage_b`). Kronos and RegimeShift use
  local snapshots in the template; replace them with live URLs for a
  live-fetch test.
- **Phase layout and filtering:** Each paid provider runs in an independent
  phase; help checks run in optional phases (help failures stay visible without
  blocking paid phases). Run `select-cases` **after filling placeholders** to
  narrow sources, cases, or reliability tags, save its JSON output, run `plan`,
  and follow [INTEGRATION.md](../INTEGRATION.md).

## Body assertions and limitations

| Provider | Required body evidence |
| --- | --- |
| Agent402 | Requested DNS name and type, plus at least one record |
| AgentFund | Nested MCP `isError=false` and content (retained timeout fails) |
| AgentUtility | No automated body assertion; manual semantic review required |
| Arkham | Balance fields present |
| Botsmith | At least one tweet |
| Brazilayer | Requested CNPJ, existence true, status field |
| Concordance | At least one result and exact matched query `Rust` (retained mismatch fails) |
| Deepline | Completed job, succeeded upstream, and output field |
| Exa and StableEnrich | At least one result |
| Genuine Good Grants | At least one opportunity |
| Glassnode | At least one asset entry |
| Google Trends | Requested keyword and at least one series entry |
| Locus | Records and summary present |
| LoneStar | Curve, credit, and as-of fields |
| Otto | Success, non-degraded flag, and report field |
| PDL | Status 200 and requested website |
| SocialFetch | Found lookup and profile field |
| Straits | Non-fallback flag and at least one bullet |
| x402stock | At least one meeting |
| RegimeShift | Overall OK and both Aave and Compound source-OK flags (retained degraded inputs fail) |
| Kronos | Non-stale flag and forecast field (cached responses allowed) |

AgentUtility deliberately omits automated success assertions because its
original signed call returned an HTTP failure. The x402-list case is unsigned
so a quota-triggered `402` fails before signing. Help caching, MCP status, body
checks, and canonical Base debits are evaluated independently.

## Offline validation and string formats

Offline preset tests verify every tool, argument, allowlist, and reservation
against `provider_requests.json`, captured remote inputs, and the seven local
production catalogs (including Kronos and RegimeShift) with no keys or network
calls.

Catalog preparation enforces `date`, `date-time`, and `uri` formats on
submitted inputs: padded calendar dates and
[RFC 3339](https://www.rfc-editor.org/rfc/rfc3339.html#section-5.6) timestamps
with explicit timezones (rejecting leap seconds), and ASCII absolute URIs with
valid escapes and a parseable authority (rejecting relative URIs and IRIs).
Unknown formats fail even in unused branches, and strings are never rewritten.
