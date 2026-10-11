# Reviewed GET alternatives to POST tools

## Objective and status

Draft, independently deliverable provider-catalog work. Review a small number of
concrete operations where an existing GET alternative could improve usability or
reduce observed initial-payer failures. Preserve useful POST behavior and public
tool compatibility. This plan authorizes no paid provider calls or funded testing.

The delivered [wallet capacity and admission](../wallet-rotation.md#capacity-and-admission)
behavior includes retained balances, runtime price hints and one unsigned payer
handoff for every supported method, including POST. Catalog curation remains
independent of those runtime rules.

## Scope and candidate selection

Start with one provider and a specific method/path pair. GenuineGood is a candidate
for investigation, not an established equivalence or a mandate to change its tools.
Choose work from observed POST selection failures, existing operator feedback, or
a documented GET alternative with a concrete usability benefit. Do not launch an
exhaustive audit of every authored provider.

Review authored TOML providers and their pinned contracts. Dynamically imported
specifications retain their existing operation selection. This work introduces no
server-specific provider implementation, method rewriting, automatic POST-to-GET
retry, extra pricing discovery or payment-policy change.

Evaluate benefits against the delivered all-method handoff. Runtime hints prefer
small sufficient capacity before binding; actual-price admission preserves a
verified sufficient bound payer. Neither hints nor handoff guarantees affordability
for changing prices. Do not remove useful POST tools to work around the former
method-specific handoff restriction. The [selection and isolation decision](../architecture.md#wallet-selection-and-network-isolation)
explains the accepted request and pooling tradeoffs.

Unsigned handoff assumes the provider gates protected execution before its unpaid 402,
as the existing unsigned-then-paid POST flow does. It is not general POST idempotency.
After signed journaling neither method gains replay, including settlement failure or
output-conversion failure. Provider-specific incompatibility needs an explicit reviewed
decision; catalog curation grants no transaction retry authority.

## Contract review and decision

For each candidate, identify exact operations and compare pinned OpenAPI definitions
with applicable provider documentation. Record the evidence and any uncertainty:

| Concern | Required comparison |
| --- | --- |
| Inputs | Parameter names/types, defaults, required values, serialization, body/query mapping and supported input-size limits |
| Semantics | Operation behavior, side effects, caching implications and whether GET is appropriate for the operation |
| Pricing | Billing rules and known argument-dependent differences; estimates alone do not prove equal cost |
| Results | Response status/content types, schemas, image mappings and any documented behavioral differences |
| Privacy | Whether moving body arguments into URLs exposes sensitive or large inputs to additional logging or handling |
| Compatibility | Existing generated names, collision naming, authored filters, listener/source selectors, examples and effective managed-group bindings; no operation-to-tier configuration |

Similar operation names or schemas alone do not prove equivalence. Offline fixtures
can establish client behavior and exposed contracts, not undocumented live provider
semantics. If evidence is missing or ambiguous, retain the existing operations and
document the limitation. Use existing evidence and unsigned documentation inspection;
do not exercise paid endpoints to establish equivalence.

Choose the smallest useful change:

1. Document an already exposed GET alternative and when to use it.
2. If useful and supported, expose the GET alternative while preserving POST.
3. Remove a POST only when a demonstrated benefit outweighs compatibility costs,
   distinct input/privacy needs are preserved, and migration guidance is concrete.

Keeping both methods is a valid final outcome. No blanket GET preference or
equivalent-POST removal requirement belongs in AGENTS.md. A review may conclude
that no catalog change is warranted.

## Implementation and documentation

Use the existing TOML selection mechanisms. When narrowing an inventory, use exact
`include_operations` entries and preserve ordinary path/tag/tool/listener filter
composition. Do not add another runtime method-preference setting.

For each changed provider, update its existing documentation or
[provider caveats](../../providers/CAVEATS.md) with reviewed method/path pairs,
supporting contract references, useful differences, retained POST cases and any
intentional exclusions. Preserve full vendor schemas and descriptions.

Before removing or adding an operation, inspect the complete generated inventory:
collision resolution can rename a different tool as well. Update authored exact
selectors, source/listener references, effective wallet/group bindings and
checked-in examples together. Explain removed/renamed tool migration; preserve
user-owned local configurations. Invalid stale selectors should retain normal
validation failures rather than receive hidden aliases or method substitution.

## Validation and acceptance

| Change | Required evidence |
| --- | --- |
| Documentation only | Accurate method/path references, explicit evidence limitations, valid links and no unsupported equivalence claim |
| Add or retain GET alongside POST | Full contract snapshots show intended methods, schemas/descriptions and names; filters and bindings still resolve |
| Remove POST | Reviewed benefit and equivalence evidence, intentional absence in snapshots, preserved non-equivalent/privacy-sensitive cases, and complete name/selector/binding migration coverage |
| Payment behavior used to justify a change | Local real-PaidClient fixtures assert the delivered version: generic all-method handoff; preserve bytes, identity/cover, caps and redirect policy; never substitute methods or replay signed work |
| Generic POST handoff | Commit-pinned middleware fixture counts protected execution: two unpaid challenges plus one signed request execute once; free second response signs nothing; signed settlement/output failure causes no retry |
| Price-hinted GET/POST comparison | Exercise actual retained-aware hint learning and fresh admission; keep a sufficient bound payer; measure handoffs, failures and retained consumption for fixed/mixed prices; no universal affordability claim |

Run focused default-Zcash provider contract/config/payment suites appropriate to the
actual change and required repository checks. Documentation-only changes need link
and consistency checks, not funded tests. Preserve existing qualification limits;
local fixtures establish no live settlement or privacy qualification.

Keep this plan consistent with the maintained wallet and selection contracts.
Provider documentation must describe actual GET/POST differences rather than implying
different group caps, reserve classes or transaction safety guarantees by method.

## Delivery

Select one concrete candidate, record its evidence in the maintained provider
documentation, and implement the smallest justified change with its fixtures and
migration guidance. Review and release each provider change independently. Expand
scope only when another concrete benefit warrants it; completion does not require
removing POST tools or reviewing all providers. Keep remaining work focused here,
then remove the completed plan under the repository's normal documentation policy.
