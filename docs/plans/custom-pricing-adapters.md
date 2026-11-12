# Custom pricing adapters

## Intent and scope

Track pricing declarations observed during the 2026-11-11 provider sweep that the
current shared adapters do not consume, and identify the providers that would need
a reviewed code change rather than a per-provider recipe. This is an implementation
plan for shared pricing normalization only. It does not authorize paid calls, live
settlement qualification, new spending limits or provider-ID dispatch.

Starting point: `c3baa63448b2a57d064a302551eab119b8630803`, plus the uncommitted
provider-update evidence/tooling changes described below. The sweep captured all
bundled providers; `ax1` and `coinmarketcap` were unavailable public reads. Only
`horizonpulse` produced a real upstream candidate. Provider evidence lives under the
ignored `target/provider-updates/` runs and is not committed.

Preserved invariants:

- Advertised prices remain advisory. Observed validated offers and the live challenge
  govern payment; no adapter changes signing, caps, wallet selection or recovery.
- Unknown is not free. Unsupported shapes stay display-only with vendor details.
- USD amounts do not become nominal USDC estimates without explicit provider
  `usd_as_usdc` opt-in and a compatible Base USDC rail.
- Metered, credit, dynamic and ambiguous tariffs remain display-only.
- Shared adapters need independent uses with synthetic tests. No provider-ID
  dispatch, executable metadata or per-provider copying.
- Existing concise `Cost:` presentation and authored `pricing_description`
  precedence are preserved.

Owning guides: [configuration](../configuration.md#pricing-metadata-and-runtime-hints),
[provider updates](../provider-updates.md#pricing-evidence-and-variation-review),
[provider policy](../../providers/README.md#provider-policy), and the
[planning workflow](../planning-workflow.md). Follow
[AGENTS.md](../../AGENTS.md) for independent review and validation order.

## Current shared adapters

`src/pricing/advertised.rs` currently supports:

- Catalog `payment-info` objects with `price`/`fixed`/explicit `min`/`max` and
  `amount`/`currency`/`unit`/network/asset wrappers.
- Catalog `x402` objects whose `price` is a dollar-prefixed string.
- External `x402-resources` and `x402-discovery` records through `accepts` payment
  requirements.
- Credit conversion through the separate `[credit_pricing]` configuration.

It does not consume a scalar price field, a bare dollar string directly under
`payment-info`, atomic `amount` plus `decimals`, or a second operation pricing key.

## Observed variations

The counts below are catalog-projection observations from the sweep. They describe
metadata coverage, not payment verification. A shape listed as display-only is
intentionally not a fixed estimate until a reviewed adapter exists.

| Provider | Observed variation | Current result | Proposed shared response |
| --- | --- | --- | --- |
| `degenscan` | `x-price-usd` numeric scalar (`0.002`, `0.001`, ...) on 120 operations; configured key is `x-price-usd` with `payment-info` | `Pricing: 0.002 [spec]`, display-only | Add a catalog-only scalar USD format that accepts a finite nonnegative number or decimal string, preserves `Cost: ~$0.002/call [spec].`, and requires `usd_as_usdc` before selection |
| `daedalus` | `x-price-usd` decimal strings (`"0.01"`, `"0.00"`) on 360 of 380 operations; only 250 also have `x-payment-info` | 86 tools unknown/missing, rest fixed | Same scalar format; migrate the provider to `x-price-usd` when the adapter is reviewed. Explicit `"0.00"` is free only under the existing explicit-zero rule |
| `relayshield` | `x-price-usd` numeric scalars on 32 metered operations without `x-payment-info`; `x-payment-info` also has `x-billing-unit` | Some metered tools unknown | Same scalar format as a fallback key for operations without the primary key; keep metered units display-only |
| `nansen` | `x-payment-info` is an object with a nested dollar-string `price` (`{"protocols":["x402","mpp"],"pricingMode":"fixed","price":"$0.05"}`); `x-credit-cost` carries credit tiers | 77 tools display-only as raw JSON | Accept a dollar-prefixed `price` string inside the payment-info object using the existing x402 string semantics (`$` stripped, USD implied) |
| `horizonpulse` | `x-payment-info` has `priceUsd` plus atomic `amount`/`decimals`, optional `networks`/`polygon`; premium operations use `x-premium-payment-info` | No `[pricing]`; 49 effective tools unknown; atomic amount must not display as major units | Read `priceUsd` when present; otherwise convert atomic `amount` with `decimals`; recognize one secondary operation key with first-present precedence; keep network alternatives display-only unless they are the same Base USDC rail |
| `agent402` | Standard `x-payment-info.price` covers most operations; `x-price` appears separately, 26 operations have no `price.amount`, and 3 effective tools are unknown | 3 unknown | No adapter required unless the unknown operations publish a concrete amount; do not add a fallback solely for these |
| `claw402` | `x-token-pricing` token schedules and `x-payment-info.price.min` dynamic tariffs | 12 display-only | No fixed-amount adapter. Keep metered/dynamic display-only; do not invent a per-call estimate |
| `brazilayer`, `halowerk` | External `x402-discovery`/`x402-resources` documents with `accepts` and advisory `price_usdc` | Catalog projection cannot show external runtime hints; external acquisition already parses `accepts` | No new adapter. The packet limitation is documented in the sweep guide; `price_usdc` need not be parsed because `accepts` is authoritative |
| `horizonpulse` discovery catalog | `https://horizonpulse.dev/.well-known/x402` uses a vendor `version`/`resources` string list plus named route maps (`sellerDd`, `paidChecks`, tiers) with dollar and atomic amounts, not x402 `resources` objects with `accepts` | Captured only as advisory pricing evidence after this sweep; no runtime adapter | Keep as sweep evidence. Do not add a one-provider runtime format; CP3 consumes the operation-level `x-payment-info`/premium records instead |
| `2s` | Duplicate `x-2s-price.usd` beside standard `x-payment-info.price` | Standard key already prices the operations | No adapter |
| `maxwell` | One explicit `min`-only range | Display-only | No adapter; incomplete ranges remain non-estimates |

`oneshot` now uses the existing standard `payment-info` adapter after the sweep added
`[pricing]` for its paid search operation; no custom adapter is needed.

## Proposed design decisions

Design review must choose names and exact semantics before implementation:

1. **Scalar format name.** Prefer `format = "usd-scalar"` over overloading
   `payment-info`. The scalar carries no currency, network or asset, so it must
   require explicit `usd_as_usdc` for a selection amount and otherwise render
   `Cost: ~$X/call [spec].` display-only. Only `source = "catalog"` is valid.
2. **Secondary keys.** Prefer an ordered `fallback_keys` list (first present wins)
   owned by the pricing configuration, rather than provider-specific dispatch or a
   second pricing table. Limit it to catalog keys and reject it for external
   sources. `horizonpulse` and `relayshield` are the concrete consumers.
3. **Atomic amounts.** Extend `payment-info` only when `priceUsd` is absent:
   require integer `amount`, integer `decimals`, a supported Base USDC asset/network,
   and a compatible unit. Never treat an atomic amount as a major-unit tariff.
   A `priceUsd` range or tier string stays display-only; it must not be coerced into
   a fixed estimate.
4. **Dollar strings.** Reuse the existing x402 dollar-string normalization for a
   nested payment-info `price` string. A bare number without a currency remains
   display-only unless a scalar format or `usd_as_usdc` explicitly interprets it.
5. **Presentation.** Keep one compact `Cost:` suffix. Dynamic, range, metered and
   credit shapes retain raw vendor detail. Authored `pricing_description` still
   wins after generated fallback.
6. **No provider recipes.** If a shape has only one provider and no likely second
   use, keep it authored in `pricing_description` or the owning caveat instead of
   adding code.

## Commit boundaries

### CP1 — Scalar USD declarations

Add the catalog-only scalar format, configuration validation, selection gating on
`usd_as_usdc`, display normalization and synthetic tests. Migrate only providers
whose evidence was reviewed: `degenscan` and `daedalus`.
Update [pricing configuration](../configuration.md#pricing-metadata-and-runtime-hints)
and the affected provider caveats. Acceptance: focused pricing unit tests, catalog
projection counts before/after, provider configuration/replay tests and unchanged
tool selection.

### CP2 — Dollar strings and secondary keys

Accept a dollar-prefixed string in the payment-info `price` field. Add ordered catalog
fallback keys with first-present precedence, no external-source use and validation
errors for malformed/duplicate entries. After exact evidence review, migrate
`nansen` and the `relayshield` scalar fallback, using CP1's scalar format for the
latter. Horizon Pulse's pricing declaration follows in CP3 with its required
normalization support. Acceptance: synthetic string/secondary-key tests,
existing x402 string parity, and no change to signing or caps.

### CP3 — Atomic amount and premium records

Read `priceUsd` when present; otherwise convert integer atomic amounts with
`decimals` for supported Base USDC. Add the secondary-key use for
`x-premium-payment-info`. Keep per-network alternatives display-only unless they are
the same Base USDC rail. Update the Horizon Pulse caveat and its pricing declaration.
Acceptance: positive `priceUsd`, atomic, mismatched-decimals, non-Base and premium
cases; original `Cost: unknown.` behavior remains for unsupported records.

Each commit includes its tests and permanent documentation. Unrelated providers and
unrelated pricing shapes stay unchanged.

## Review gates

- Before CP1, an independent read-only design review must challenge the scalar
  format, `usd_as_usdc` requirement, fallback-key shape and whether any variation
  belongs in authored prose instead.
- Independently review each exact commit SHA and fixup. The reviewer must not be the
  author and must inspect the complete pricing parse, catalog projection, selection
  hint, description rendering and existing credit/external paths.
- A combined integration gate reviews all commits and fixups, compares source-matched
  CogC/CycC against the starting baseline, and checks that no provider dispatch,
  unnecessary enforcement or per-provider routine emerged.
- Revalidation follows [validation order](../testing.md#validation-order-and-continuation).
  Run affected cheap pricing/catalog tests first, then compiler and all-target
  Clippy when selected. No paid live calls are authorized.

## Acceptance evidence

- Provider sweeps show the intended improvement:
  `degenscan` scalar operations display fixed estimates; `daedalus` no longer leaves
  `x-price-usd`-only operations unknown; `nansen` dollar strings render fixed;
  `horizonpulse` atomic/premium records render correct major-unit prices only when
  supported.
- Unsupported metered/credit/dynamic/ambiguous shapes remain display-only or unknown.
- No change to challenge validation, payment authority, caps, wallet selection,
  funding, signing or journaling.
- Maintained documentation names the actual supported formats and opt-ins, and the
  [provider update guide](../provider-updates.md#pricing-coverage-triage-and-adapter-gaps)
  points here for unresolved shapes.
- The completed plan is removed from the tree only after the handoff and exact-SHA
  cleanup review.
