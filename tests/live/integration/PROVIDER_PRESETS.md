# Reviewed provider sweep preset

`providers.example.toml` defines the reviewed 23-provider baseline.
`../fixtures/provider_requests.json` independently pins its paid request contracts.
It contains the same 22 paid requests and their $0.20 reservations, plus an
unsigned x402-list directory request. It does not include OneShot or add newly
discovered providers implicitly. Additional routes, concurrency and depletion calls require separate reviewed scenarios.
There are 63 explicit cases: 22 paid, one directory read and 40 unsigned help calls
(fetch/hit pairs for the 20 configured help tools).

The zero time window and placeholder treasury ID prohibit execution until the
operator reviews the deployment, state, run ID, deadlines and authority. Copy the
referenced `examples/live-tor-providers.toml` to a private deployment, configure
the already-funded treasury/pools and owned Tor endpoint, then update the manifest
paths. Relative paths in the committed template are relative to its committed
location; use absolute paths when relocating a copy. The API reservation ceiling
is $4.40, not a fee estimate. All new funding/job authority is zero, auto-funding
is disabled in the deployment, and paid cases select pool `coverage_b`. Other
declared pools remain configured; the preset does not remove or fund them.

Each paid provider has an independent phase with no dependencies on other
providers or help success. Available help tools have unsigned fetch/hit checks in
optional phases. Startup pricing is asserted disabled for the 23 selected sources.
Help assertion failures remain visible and can leave overall qualification
incomplete; independent paid phases still proceed.
This is the probe-disabled baseline; it does not replace the separate required
empty/nonempty pricing and wallet-isolation acceptance cases. Tor and cover policy
are inherited from the reviewed deployment. Kronos and RegimeShift currently use
explicit local snapshots in that template; their loading cannot qualify live
catalog availability. Replace those references explicitly for a live-fetch run.
Startup still requires the complete configured inventory; case selection alone
does not remove a provider from startup.

Run `select-cases` to select exact sources/cases or exclude configured reliability
tags from this new-format manifest **after filling its placeholders**. Preserve
the selection JSON as review evidence. Then run `plan`, inspect reservations and
bindings, and use the normal preparation, registry and owned-Tor execution flow
in [the runner guide](../INTEGRATION.md). Selection does not grant permission,
refresh a deadline, reset reservations or replay a prior case.

## Body assertions and limitations

The body checks below are pinned response assertions. They are minimal
response contracts, not financial correctness, freshness or relevance guarantees
beyond the explicit checks. Existing registry results are not rewritten.

| Provider | Required body evidence |
| --- | --- |
| Agent402 | Requested DNS name/type and at least one record |
| AgentFund | Nested MCP `isError=false` and content; the retained timeout fails |
| AgentUtility | No successful body contract observed; manual semantic review remains required |
| Arkham | Balance fields present |
| Botsmith | At least one tweet |
| Brazilayer | Requested CNPJ, existence true, status field |
| Concordance | At least one result and exact reviewed matched query `Rust`; the retained mismatch fails |
| Deepline | Completed job, succeeded upstream and output field |
| Exa / StableEnrich | At least one result |
| Genuine Good Grants | At least one opportunity |
| Glassnode | At least one asset entry |
| Google Trends | Requested keyword and at least one series entry |
| Locus | Records and summary present |
| LoneStar | Curve, credit and as-of fields |
| Otto | Success, non-degraded flag and report field |
| PDL | Status 200 and requested website |
| SocialFetch | Found lookup and profile field |
| Straits | Non-fallback flag and at least one bullet |
| x402stock | At least one meeting |
| RegimeShift | Overall OK and both Aave/Compound source-OK flags; retained degraded inputs fail |
| Kronos | Non-stale flag and forecast field; cache use itself is allowed |

The original AgentUtility signed call returned an HTTP failure. Its preset keeps
the request but deliberately omits invented success assertions, so an eventual
HTTP success cannot pass provider semantics without review. The directory case
is explicitly unsigned: a quota-triggered 402 fails before signing. Help caching,
outer MCP success, body checks and canonical payment debit remain separate gates.

The offline preset test checks the exact paid request/argument/reservation mapping
against the older reviewed input, validates all tools and arguments against committed
catalogs and listener allowlists, and verifies $4.40 reservations with no new funding.
It needs no keys, state or live endpoints. Local synthetic tests cover assertion
evaluation; none of these checks claim a fresh funded or Tor-qualified sweep.

Catalog preparation asserts the `date`, `date-time` and `uri` formats used by the
provider inputs. Date/date-time checks require padded calendar dates and [RFC3339](https://www.rfc-editor.org/rfc/rfc3339.html#section-5.6)
timestamps with an explicit timezone. URI qualification accepts ASCII absolute
references with valid escapes and a parseable authority. Leap-second timestamps
are outside this qualification subset and are rejected; the runner does not guess
whether a declared leap second is valid. Relative URI references and IRIs are
refused. These are conservative qualification checks, not a claim to
implement every JSON Schema format. Unknown formats still fail, including on
unused properties and alternative branches. Submitted strings are not rewritten.
