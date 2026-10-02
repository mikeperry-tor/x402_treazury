#!/usr/bin/env python3
"""Build a compact operations digest from the Social Fetch OpenAPI spec.

Usage:
    python scripts/build_socialfetch_digest.py [openapi.json|-URL-] [out.json]

Strips response schemas (which make the raw spec ~24MB) and keeps only what
is needed to generate MCP tool definitions: path, method, summary,
description, pricing extensions, parameters, and request body schemas.
"""

from __future__ import annotations

import json
import sys
import urllib.request

MAX_DESC = 400

KEEP_SCALAR_KEYS = (
    "type",
    "format",
    "enum",
    "const",
    "minimum",
    "maximum",
    "exclusiveMinimum",
    "exclusiveMaximum",
    "minLength",
    "maxLength",
    "minItems",
    "maxItems",
    "pattern",
    "default",
    "nullable",
)


def trim_desc(value):
    if isinstance(value, str) and len(value) > MAX_DESC:
        return value[: MAX_DESC - 1].rstrip() + "…"
    return value


def minify_schema(s, depth: int = 0):
    if not isinstance(s, dict):
        return s
    out = {}
    for k in KEEP_SCALAR_KEYS:
        if k in s:
            out[k] = s[k]
    if "description" in s:
        out["description"] = trim_desc(s["description"])
    if "title" in s:
        out["title"] = s["title"]
    if depth < 8:
        if "items" in s:
            out["items"] = minify_schema(s["items"], depth + 1)
        if "properties" in s:
            out["properties"] = {
                k: minify_schema(v, depth + 1) for k, v in s["properties"].items()
            }
        if "required" in s and isinstance(s["required"], list):
            out["required"] = s["required"]
        ap = s.get("additionalProperties")
        if isinstance(ap, dict):
            out["additionalProperties"] = minify_schema(ap, depth + 1)
        elif isinstance(ap, bool):
            out["additionalProperties"] = ap
        for comb in ("anyOf", "oneOf", "allOf"):
            if comb in s:
                out[comb] = [minify_schema(v, depth + 1) for v in s[comb][:6]]
    return out


def main() -> None:
    src = sys.argv[1] if len(sys.argv) > 1 else "https://www.socialfetch.dev/openapi.json"
    out = sys.argv[2] if len(sys.argv) > 2 else "src/x402_mcp/data/socialfetch_openapi_digest.json"

    if src.startswith("http"):
        req = urllib.request.Request(src, headers={"User-Agent": "x402-mcp-digest/0.1"})
        raw = urllib.request.urlopen(req, timeout=120).read()
        spec = json.loads(raw)
    else:
        with open(src) as f:
            spec = json.load(f)

    operations = []
    for path, ops in sorted(spec.get("paths", {}).items()):
        for method, op in ops.items():
            if method not in ("get", "post", "put", "patch", "delete"):
                continue
            entry = {
                "path": path,
                "method": method,
                "summary": op.get("summary"),
                "description": trim_desc(op.get("description")),
                "pricing": op.get("x-socialfetch-credits-pricing"),
            }
            entry["params"] = []
            for p in op.get("parameters", []):
                schema = minify_schema(p.get("schema", {}))
                desc = trim_desc(p.get("description") or schema.get("description"))
                if schema.get("description") == desc:
                    schema.pop("description", None)
                entry["params"].append(
                    {
                        "name": p.get("name"),
                        "in": p.get("in"),
                        "required": bool(p.get("required")),
                        "description": desc,
                        "schema": schema,
                    }
                )
            rb = op.get("requestBody")
            if rb:
                content = rb.get("content", {}).get("application/json", {})
                entry["body"] = {
                    "required": bool(rb.get("required")),
                    "schema": minify_schema(content.get("schema", {})),
                }
            else:
                entry["body"] = None
            operations.append(entry)

    digest = {
        "meta": {
            "title": spec.get("info", {}).get("title"),
            "version": spec.get("info", {}).get("version"),
            "source": "https://www.socialfetch.dev/openapi.json",
            "operation_count": len(operations),
        },
        "operations": operations,
    }

    with open(out, "w") as f:
        json.dump(digest, f, separators=(",", ":"), sort_keys=True)
    print(f"wrote {out}: {len(operations)} operations")


if __name__ == "__main__":
    main()
