"""LoneStarOracle (lonestaroracle.xyz) as MCP tools, pay-per-call with x402.

LoneStarOracle is ~71 independent pay-per-call data services, each on its own
subdomain (<service>.lonestaroracle.xyz) with its own OpenAPI spec. This
launcher merges them into ONE MCP server: tools are generated from a committed
digest of all service specs (scripts/build_lonestar_digest.py) and grouped the
way the vendor's catalog groups them. Each tool carries its absolute
per-service URL, so one wallet/transport serves every subdomain:

    x402-mcp-lonestar --groups macro,crypto
    x402-mcp-lonestar --groups token,rates     # individual services work too
    x402-mcp-lonestar --groups all
    x402-mcp-lonestar --list-groups

Every route settles per call via x402 USDC on Base; each tool's description
states its price (from the vendor specs' x-payment-info), and genuinely free
routes say so.
"""

from __future__ import annotations

import argparse
import json
import sys
from importlib.resources import files

from .digest import sanitize_schema, slug_suffix
from .runner import ToolSpec, serve

# Nominal default for serve()'s --base-url. Every generated tool carries an
# absolute https://<service>.lonestaroracle.xyz/... path, which httpx sends
# as-is; the base URL is never actually prefixed onto a request.
BASE_URL = "https://lonestaroracle.xyz"

# Service slug -> docs-catalog group. Slugs are the subdomain labels; every
# service lives at https://<slug>.lonestaroracle.xyz. Groups mirror
# https://docs.lonestaroracle.xyz/#catalog (the vendor's own counts in the
# section headers are off by one in places; the rows are authoritative).
GROUPS: dict[str, list[str]] = {
    # "Macro & Real Economy" in the vendor catalog; named real-economy here
    # so the group key never collides with the `macro` service slug.
    "real-economy": [
        "macro", "rates", "liquidity", "labor", "consumer", "cycle", "trade",
        "supply", "metals", "minerals", "energy", "spark", "computeindex",
        "grid", "compute", "agri", "realestate", "crownblock", "lease",
    ],
    "equities": [
        "equity", "ta", "options", "earnings", "analyst", "smartmoney",
        "insider", "capitol", "funds", "cot", "squeeze", "pharma",
        "portfolio", "wealth",
    ],
    "crypto": [
        "token", "chainscout", "wallet", "contract", "launches", "defi",
        "rwa", "whale", "stable", "stake", "funding", "oi", "liq", "bundle",
        "pnl",
    ],
    "risk": [
        "sanctions", "kya", "geo", "conflict", "cascade", "sovereign",
        "hazard", "mena", "asia", "europe", "africa", "latam",
    ],
    "audits": ["rattler", "cottonmouth", "copperhead"],
    "govnews": [
        "govedge", "news", "weather", "aero", "doc", "read", "content",
        "floyd",
    ],
}

ALL_SERVICES = sorted({s for services in GROUPS.values() for s in services})

INSTRUCTIONS = (
    "LoneStarOracle: ~71 independent pay-per-call data services merged into "
    "one server — macro/labor/energy, equities and smart-money flow, crypto "
    "due diligence, geopolitical/compliance risk, and contract audits. Each "
    "tool calls its own <service>.lonestaroracle.xyz host. Everything settles "
    "per call via x402 USDC on Base; the price is stated in every tool "
    "description (free routes say 'Free'). Answers are signal-first JSON, "
    "not raw data dumps. Contract audits can take minutes — raise --timeout "
    "for the audits group."
)


def load_digest() -> dict:
    raw = files("x402_mcp.data").joinpath("lonestar_openapi_digest.json").read_text()
    return json.loads(raw)


def resolve_services(tokens: list[str]) -> list[str]:
    """Expand --groups tokens (group names, service slugs, or 'all') into an
    ordered, de-duplicated service list."""
    expanded: list[str] = []
    for token in tokens:
        key = token.strip().lower().replace("_", "-")
        if key == "all":
            expanded.extend(ALL_SERVICES)
        elif key in GROUPS:
            expanded.extend(GROUPS[key])
        elif key in ALL_SERVICES:
            expanded.append(key)
        else:
            raise SystemExit(
                f"unknown group or service {token!r}. Groups: "
                f"{', '.join(sorted(GROUPS))}. Services: "
                f"{', '.join(ALL_SERVICES)} (also: 'all')"
            )
    return list(dict.fromkeys(expanded))


def service_url(service: str) -> str:
    return f"https://{service}.lonestaroracle.xyz"


def build_tools(services: list[str]) -> list[ToolSpec]:
    selected = set(services)
    unknown = selected - set(ALL_SERVICES)
    if unknown:
        raise RuntimeError(
            f"digest references services absent from the catalog: {sorted(unknown)}"
        )
    digest = load_digest()
    tools: list[ToolSpec] = []
    seen: set[str] = set()
    for op in digest["operations"]:
        if op["service"] not in selected:
            continue
        # launcher-wide prefix: every LoneStarOracle tool starts with
        # lonestar_, then the service slug, then the route slug
        name = f"lonestar_{op['service']}_{slug_suffix('', op['path'])}"
        if name in seen:
            raise RuntimeError(f"duplicate tool name {name}")
        seen.add(name)
        tools.append(_tool_from_op(name, op))
    return tools


def _pricing_line(op: dict) -> str:
    pricing = op.get("pricing")
    if pricing is None:
        return (
            "Paid per call via x402 (price set by the API; per-payment cap "
            "via X402_MAX_PRICE_USD)."
        )
    if pricing.get("free") is True:
        return "Free — no payment required."
    amount = pricing.get("amount")
    currency = str(pricing.get("currency") or "").upper()
    if amount is not None and currency in ("USD", "USDC"):
        return f"Price: ${amount} per call (vendor spec)."
    # Unknown shape: keep the vendor data visible, never invent a rate.
    compact = json.dumps(pricing, separators=(",", ":"), sort_keys=True)
    return f"Pricing: {compact} (vendor spec)."


def _tool_from_op(name: str, op: dict) -> ToolSpec:
    properties: dict[str, dict] = {}
    required: list[str] = []
    routes: dict[str, str] = {}

    for p in op.get("params") or []:
        if p["in"] == "header":
            continue
        prop = dict(p.get("schema") or {"type": "string"})
        if p.get("description") and "description" not in prop:
            prop["description"] = p["description"]
        prop.setdefault("type", "string")
        prop_name = p["name"]
        properties[prop_name] = prop
        routes[prop_name] = "path" if p["in"] == "path" else "query"
        if p.get("required"):
            required.append(prop_name)

    has_body = op.get("body") is not None
    if has_body:
        body = op["body"]["schema"]
        for k, v in (body.get("properties") or {}).items():
            prop_name = k if k not in properties else f"{k}_body"
            if prop_name in properties:
                raise RuntimeError(f"unresolvable body/query collision in {op['path']}: {k}")
            properties[prop_name] = v if isinstance(v, dict) else {"type": "string"}
            routes[prop_name] = "body"
        for k in body.get("required") or []:
            required.append(k if k in properties else f"{k}_body")

    input_schema: dict = {"type": "object", "properties": properties}
    if required:
        input_schema["required"] = sorted(set(required))
    input_schema = sanitize_schema(input_schema)

    # One vendor text, never both: the long `description` supersedes the
    # `summary` stub. Falls back to summary, then METHOD + full URL.
    url = service_url(op["service"]) + op["path"]
    head = op.get("description") or op.get("summary") or f"{op['method'].upper()} {url}"
    description = " ".join([head, _pricing_line(op)])

    return ToolSpec(
        name=name,
        description=description,
        method=op["method"].upper(),
        path=url,  # absolute: routes to the service's own subdomain
        input_schema=input_schema,
        param_routes=routes,
        has_body=has_body,
    )


def _groups_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(add_help=False)
    parser.add_argument(
        "--groups",
        default=None,
        help="Comma-separated catalog groups and/or individual service slugs. "
        "See --list-groups. Use 'all' for everything.",
    )
    parser.add_argument(
        "--list-groups", action="store_true", help="List group selectors and exit"
    )
    return parser


def main(argv: list[str] | None = None) -> None:
    argv = list(sys.argv[1:] if argv is None else argv)
    pre_args, rest = _groups_parser().parse_known_args(argv)

    if pre_args.list_groups:
        print("groups (use with --groups, comma-separated; service slugs work too):")
        for key in sorted(GROUPS):
            print(f"  {key:10} ({len(GROUPS[key])}): " + " ".join(GROUPS[key]))
        print("  all        (" + str(len(ALL_SERVICES)) + "): every service")
        return

    if not pre_args.groups:
        _groups_parser().error("--groups is required (see --list-groups)")

    tokens = [t.strip() for t in pre_args.groups.split(",") if t.strip()]
    services = resolve_services(tokens)
    tools = build_tools(services)
    if not tools:
        _groups_parser().error(f"--groups {pre_args.groups!r} matched no endpoints")

    serve(
        f"x402 LoneStarOracle [{','.join(tokens)}]",
        "0.1.0",
        tools,
        default_base_url=BASE_URL,
        default_timeout=120.0,
        argv=rest,
        instructions=INSTRUCTIONS,
    )


if __name__ == "__main__":
    main(sys.argv[1:])
