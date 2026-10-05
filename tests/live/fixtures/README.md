# Selected request fixtures

These four snapshots contain only the selected paths, plus original OpenAPI
metadata, servers and components, from the provider audit captures used during
2026-10-04 preparation. They supplement existing catalog fixtures for Arkham,
botsmith, Google Trends and x402stock; response declarations are retained.

`live_integration/presets.rs` uses these exclusively for offline input and
scope validation. Live deployment templates continue to fetch the configured
remote specifications. These snapshots cannot substitute for a failed live spec
or establish current API prices, availability or payment compatibility.

`provider_requests.json` independently pins the 22 original reviewed paid requests:
case ID, tool, exact arguments and reservation. It was extracted from the original
driver input, not generated from the current preset. The preset regression compares
both sources; changing either requires deliberate request and budget review.
