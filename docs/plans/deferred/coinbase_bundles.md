# Coinbase bundle discovery

Bundle listing and ordered bundle details remain outside the implemented Coinbase
resource-search interface. Agent search exposes no bundle filters, enumeration or
membership/order/completeness claims.

The reviewed Coinbase
[bundle list](https://docs.cdp.coinbase.com/api-reference/v2/rest-api/x402-facilitator/list-x402-bundles)
and [bundle detail](https://docs.cdp.coinbase.com/api-reference/v2/rest-api/x402-facilitator/get-an-x402-bundle)
endpoints returned HTTP 401 in unauthenticated direct tests. Tor requests encountered
a Cloudflare challenge. The reviewed documentation showed bearer authentication,
but did not explain the distinction from public resource search. Authentication
intent, the required key type/permissions and successful authenticated access remain
unverified. No x402-paid bundle-access alternative was found in those reviewed
documents. Curl bundle access was not tested.

Before proposing bundle tools:

- Verify the current public/authenticated contract and required credentials through
  authoritative documentation or a separately authorized test.
- Establish a usable transport and exact response, ordering and completeness
  contracts; resource-search results cannot stand in for ordered bundle details.
- Review endpoint ownership, credential handling, Tor/relay trust and any payment
  authority independently of resource search. Configured search relay authority grants
  no bundle-fetch or CDP credential authority.
- Define the smallest interface that serves the verified contract, with independent
  design/clarity review and focused regressions. Do not add speculative workflow
  installation, MCP execution or guessed bundle schemas.

This deferred work authorizes no credential acquisition, live paid calls or wallet
access. The maintained [agent source guide](../../agent-sources.md) and
[relay configuration](../../configuration.md#relay-fallback) own implemented
resource search and its current boundaries.
