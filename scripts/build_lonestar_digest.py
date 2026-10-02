#!/usr/bin/env python3
"""Build the LoneStarOracle operations digest from the vendor's unified spec.

Usage:
    python scripts/build_lonestar_digest.py [out.json]

LoneStarOracle publishes ONE unified catalog spec at
https://lonestaroracle.xyz/openapi.json (root `servers` at the /api gateway,
per-op `x-direct-url` naming the canonical subdomain, `x-payment-info` with
`authMode: payment|free`). This script fetches it once, splits each path into
(service slug, per-service path), normalizes pricing from x-payment-info, and
emits the shared digest shape (plus a `service` field) that the launcher
merges into one MCP server.

The emitted digest must stay byte-identical across refactors (sort_keys,
compact separators, hardcoded meta.source) — it is committed to the repo.
"""

from __future__ import annotations

import json
import sys
import urllib.request

from x402_mcp.digest import minify_schema
from x402_mcp.lonestar import ALL_SERVICES, GROUPS, service_url

DEFAULT_OUT = "src/x402_mcp/data/lonestar_openapi_digest.json"
UNIFIED_SPEC_URL = "https://lonestaroracle.xyz/openapi.json"
SPEC_UA = "x402-mcp-digest/0.1"

# Routes never exposed as tools: vendor index/liveness/interactive or
# subscription-management routes, demos/previews, and unpriced extras the
# catalog does not document. Matched as exact paths or path prefixes.
EXCLUDED_ROUTES = (
    "/",
    "/health",
    "/demo",
    "/preview",
    "/subscribe",
    "/subscribers",
    "/contests",
    "/immunefi",
    "/targets",
    "/feed",
    "/refinery",
    "/report/text",
    "/aim/webhook",
    # LeaseEdge extras: x402-gated but publish no price (no challenge header,
    # body just says "payment required") and the catalog does not document
    # them — unverifiable pricing, so excluded.
    "/auctions",
    "/expiring",
    "/portfolio",
)

# (service, path) -> pricing for kept routes that cannot be auto-priced:
# templated GET paths (a probe would need a valid placeholder value), GET
# routes whose free/paid status must not depend on a live probe, and POST
# routes (never probed unpaid — they may have side effects). Empty since
# 2026-09-30: the unified spec classifies every op (the former entries —
# crownblock /history free, stable /symbol $0.05, floyd /status free — are
# now expressed by the vendor's own x-payment-info). Kept as a mechanism for
# future build-time corrections.
FIXUPS: dict[tuple[str, str], dict] = {}

USDC_ASSETS = frozenset(
    {"usdc", "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913"}  # USDC on Base
)


def _probe_pricing(service: str, path: str) -> dict | None:
    """Price an unpriced kept GET route from its live 402 challenge.

    Plain unpaid GET — the same discipline as the generic launcher's boot
    probe; nothing is ever signed. A 402 + PAYMENT-REQUIRED challenge is the
    vendor's authoritative price (spec x-payment-info presence flaps between
    spec fetches, the gate does not). 200 = ungated = free. Returns None
    when inconclusive (caller keeps pricing null and warns).
    """
    import base64

    url = service_url(service) + path
    req = urllib.request.Request(url, headers={"User-Agent": SPEC_UA})
    try:
        with urllib.request.urlopen(req, timeout=20) as resp:
            if 200 <= resp.status < 300:
                return {"free": True}
            return None
    except urllib.error.HTTPError as e:
        raw = e.headers.get("PAYMENT-REQUIRED") or e.headers.get("X-PAYMENT-REQUIRED")
        if not raw:
            return None
        try:
            challenge = json.loads(base64.b64decode(raw))
            accepts = challenge.get("accepts") or []
            first = accepts[0] if accepts else {}
        except Exception:
            return None
        atomic = first.get("amount") or first.get("maxAmountRequired")
        asset = str(first.get("asset") or "")
        if atomic is None or not asset:
            return None
        if asset.lower() in USDC_ASSETS:
            usd = int(atomic) / 1e6
            text = f"{usd:.2f}" if round(usd, 2) == usd else f"{usd:.6f}".rstrip("0")
            return {"amount": text, "currency": "USD"}
        return {"amount_atomic": str(atomic), "asset": asset}


def _fetch_unified_spec() -> dict:
    req = urllib.request.Request(UNIFIED_SPEC_URL, headers={"User-Agent": SPEC_UA})
    with urllib.request.urlopen(req, timeout=60) as resp:
        return json.loads(resp.read())


def _split_unified_path(path: str) -> tuple[str, str] | None:
    """Split a unified-spec path into (service slug, per-service path).

    "/aero/pool" -> ("aero", "/pool"); the catalog landing "/" -> None.
    The first segment doubles as the subdomain label (verified against every
    op's x-direct-url host at build time).
    """
    segments = path.strip("/").split("/", 1)
    if not segments or not segments[0]:
        return None
    service = segments[0]
    sub_path = "/" + segments[1] if len(segments) > 1 else "/"
    return service, sub_path


def _excluded(path: str) -> bool:
    return any(path == x or path.startswith(x + "/") for x in EXCLUDED_ROUTES)


def _inline_refs(schema: dict, components: dict, seen: frozenset) -> dict:
    """Deep-copy a schema with local $ref pointers replaced by their target
    schemas (cycle-guarded)."""
    if not isinstance(schema, dict):
        return schema
    ref = schema.get("$ref")
    if isinstance(ref, str) and ref.startswith("#/components/schemas/"):
        name = ref.split("/")[-1]
        target = components.get(name)
        if not isinstance(target, dict) or name in seen:
            return {"type": "object"}
        return _inline_refs(target, components, seen | {name})
    out = {}
    for k, v in schema.items():
        if isinstance(v, dict):
            out[k] = _inline_refs(v, components, seen)
        elif isinstance(v, list):
            out[k] = [
                _inline_refs(i, components, seen) if isinstance(i, dict) else i
                for i in v
            ]
        else:
            out[k] = v
    return out


def _money(text) -> str:
    """Canonical amount text: trailing zeros trimmed but at least two
    decimals ("0.050000" -> "0.05", "1.000000" -> "1.00", "0.10" stays)."""
    whole, _, frac = str(text).partition(".")
    frac = frac.rstrip("0").ljust(2, "0")
    return f"{whole}.{frac}"


def _normalize_pricing(op: dict, service: str, path: str) -> dict | None:
    info = op.get("x-payment-info")
    if isinstance(info, dict):
        price = info.get("price")
        if isinstance(price, dict) and price.get("amount") is not None:
            return {"amount": _money(price["amount"]), "currency": "USD"}
        if info.get("authMode") == "free":
            return {"free": True}
    if (service, path) in FIXUPS:
        return dict(FIXUPS[(service, path)])
    return None


def main() -> None:
    out = sys.argv[1] if len(sys.argv) > 1 else DEFAULT_OUT

    spec = _fetch_unified_spec()

    operations = []
    warnings: list[str] = []
    probed = 0
    services_seen: set[str] = set()
    for path, path_ops in sorted(spec.get("paths", {}).items()):
        split = _split_unified_path(path)
        if split is None:
            continue
        service, sub_path = split
        direct = None
        for method, op in path_ops.items():
            if method in ("get", "post") and op.get("x-direct-url"):
                direct = op["x-direct-url"]
                break
        if direct:
            expected = f"https://{service}.lonestaroracle.xyz"
            if not direct.startswith(expected + "/") and direct != expected:
                warnings.append(
                    f"x-direct-url host mismatch: {path} -> {direct}"
                )
        for method, op in sorted(path_ops.items()):
            if method not in ("get", "post", "put", "patch", "delete"):
                continue
            if service not in ALL_SERVICES:
                warnings.append(
                    f"service {service!r} not in the launcher catalog; skipped "
                    f"{method.upper()} {path} (add it to GROUPS and regenerate)"
                )
                continue
            services_seen.add(service)
            if _excluded(sub_path):
                continue
            pricing = _normalize_pricing(op, service, sub_path)
            if pricing is None and (service, sub_path) in FIXUPS:
                pricing = dict(FIXUPS[(service, sub_path)])
            if pricing is None and method == "get" and "{" not in sub_path:
                pricing = _probe_pricing(service, sub_path)
                probed += 1
                if pricing is not None:
                    print(f"probed {service} {sub_path}: {pricing}")
            if pricing is None:
                warnings.append(f"unpriced route kept: {service} {method.upper()} {sub_path}")
            components = spec.get("components", {}).get("schemas", {})
            entry = {
                "service": service,
                "path": sub_path,
                "method": method,
                "summary": op.get("summary"),
                "description": op.get("description"),
                "pricing": pricing,
                "params": [],
                "body": None,
            }
            for p in op.get("parameters", []) or []:
                schema = minify_schema(
                    _inline_refs(p.get("schema", {}), components, frozenset())
                )
                entry["params"].append(
                    {
                        "name": p.get("name"),
                        "in": p.get("in"),
                        "required": bool(p.get("required")),
                        "description": p.get("description"),
                        "schema": schema,
                    }
                )
            rb = op.get("requestBody")
            if rb:
                body_schema = _inline_refs(
                    rb.get("content", {}).get("application/json", {}).get("schema", {}),
                    components,
                    frozenset(),
                )
                entry["body"] = {
                    "required": bool(rb.get("required")),
                    "schema": minify_schema(body_schema),
                }
            operations.append(entry)

    operations.sort(key=lambda o: (o["service"], o["path"], o["method"]))
    digest = {
        "meta": {
            "title": "LoneStarOracle",
            "source": (
                "https://lonestaroracle.xyz/openapi.json unified catalog, "
                "fetched 2026-09-30"
            ),
            "service_count": len(services_seen),
            "operation_count": len(operations),
        },
        "operations": operations,
    }
    for w in sorted(set(warnings)):
        print("WARNING:", w, file=sys.stderr)
    print(f"probed {probed} unpriced GET route(s) against the live 402 gates")
    with open(out, "w") as f:
        json.dump(digest, f, separators=(",", ":"), sort_keys=True)
    print(f"wrote {out}: {len(operations)} operations across {len(services_seen)} services")


if __name__ == "__main__":
    main()
