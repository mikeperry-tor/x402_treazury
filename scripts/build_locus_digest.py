#!/usr/bin/env python3
"""Build the Locus operations digest from the vendor OpenAPI spec.

Usage:
    python scripts/build_locus_digest.py [openapi.json|-URL-] [out.json]

The vendor spec (https://api.locus.report/openapi.json, ~1.8MB) carries three
route families:

- `/api/locus-<slug>` (hyphen) — paid POST endpoints, each with a correct-shape
  `x-payment-info.price` block. These become the digest's paid operations with
  pricing embedded, so boots make zero probe requests (POSTs are never probed).
- `/api/locus_<slug>` (underscore) — free REST duals (`charged: false`). Their
  path slugs collide with the paid hyphen twins under path-derived tool naming
  (`locus-flood-zone` and `locus_flood_zone` slugify identically), so they are
  NOT mounted; the free tier rides the `/tools/call` gateway instead, which is
  the shape of the vendor's own remote MCP (search/execute pair).
- `/tools/list` + `/tools/call` — the free surface. `call` declares its body as
  a oneOf of 114 `{name: {const}, arguments}` wrappers, which the generic
  flattener cannot expose; the digest rewrites it as one flat gateway body
  (`name` enum of all 114 consts + `arguments` object) and mounts `list` with
  its documented `compact`/`category` query knobs as parameters.

Three pollable-job routes are excluded on purpose (`EXCLUDED_PATHS`): they
return a job descriptor (statusUrl/webhookUrl) an MCP agent cannot poll, and
the retrieval route is not in the spec — exposing them would invite
pay-with-no-result calls.

The emitted digest must stay byte-identical across rebuilds from the same
spec (sort_keys, compact separators, hardcoded meta.source) — it is committed
to the repo and pinned by tests/test_confs.py against tests/fixtures/.
"""

from __future__ import annotations

import json
import sys

from x402_mcp.digest import build_operations, load_spec

SPEC_URL = "https://api.locus.report/openapi.json"
DEFAULT_OUT = "confs/locus/locus_digest.json"
PRICING_KEY = "x-payment-info"

EXCLUDED_PATHS = {
    "/api/locus-place-report-batch",
    "/api/locus-record-batch",
    "/api/locus-property-update",
}

FREE_NOTE = "Free — no payment required ($0); this server never charges for it."

ZERO_PRICE = {"price": {"amount": "0", "currency": "USD", "mode": "fixed"}}


def _price_block(op: dict) -> dict:
    # build_operations(pricing_key=...) already copied the whole extension
    # into the entry's "pricing"; narrow it to the renderable price block.
    info = op.get("pricing") or {}
    price = info.get("price")
    if not isinstance(price, dict) or not isinstance(price.get("amount"), str):
        raise ValueError(f"paid op {op['path']} lacks a usable {PRICING_KEY}.price")
    return {k: price[k] for k in ("amount", "currency", "mode") if k in price}


def _gateway_ops(spec: dict) -> list[dict]:
    list_src = spec["paths"]["/tools/list"]["get"]
    call_src = spec["paths"]["/tools/call"]["post"]
    body = call_src["requestBody"]["content"]["application/json"]["schema"]
    consts = sorted(
        branch["properties"]["name"]["const"]
        for branch in body["oneOf"]
        if isinstance(branch.get("properties", {}).get("name", {}).get("const"), str)
    )
    if not consts:
        raise ValueError("no free-tool name consts found in /tools/call oneOf")

    list_op = {
        "path": "/tools/list",
        "method": "get",
        "summary": list_src.get("summary"),
        "description": (list_src.get("description") or "").rstrip() + " " + FREE_NOTE,
        "pricing": dict(ZERO_PRICE),
        "tags": [t for t in list_src.get("tags") or [] if isinstance(t, str)],
        "params": [
            {
                "name": "compact",
                "in": "query",
                "required": False,
                "description": "Omit input schemas for a small-context catalog (recommended first call).",
                "schema": {"type": "boolean"},
            },
            {
                "name": "category",
                "in": "query",
                "required": False,
                "description": "Filter the catalog by category (e.g. environmental, policy).",
                "schema": {"type": "string"},
            },
        ],
        "body": None,
    }
    call_op = {
        "path": "/tools/call",
        "method": "post",
        "summary": call_src.get("summary"),
        "description": (
            (call_src.get("description") or "").rstrip()
            + " Pick `name` from the locus_list catalog and pass that tool's "
            + "input object as `arguments`. "
            + FREE_NOTE
        ),
        "pricing": dict(ZERO_PRICE),
        "tags": [t for t in call_src.get("tags") or [] if isinstance(t, str)],
        "params": [],
        "body": {
            "required": True,
            "schema": {
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "enum": consts,
                        "description": "A free tool name from the locus_list catalog.",
                    },
                    "arguments": {
                        "type": "object",
                        "description": (
                            "That free tool's input object; per-tool input schemas "
                            "come from the locus_list catalog."
                        ),
                    },
                },
                "required": ["name", "arguments"],
            },
        },
    }
    return [list_op, call_op]


def build_digest(spec: dict) -> dict:
    ops = build_operations(spec, pricing_key=PRICING_KEY, include_tags=True)
    paid = []
    for op in ops:
        path = op["path"]
        if not path.startswith("/api/locus-"):
            continue
        if path in EXCLUDED_PATHS:
            continue
        op = dict(op)
        op["pricing"] = _price_block(op)
        paid.append(op)
    if not paid:
        raise ValueError("no paid /api/locus-* operations found in spec")
    operations = paid + _gateway_ops(spec)
    return {
        "meta": {
            "title": spec.get("info", {}).get("title"),
            "version": spec.get("info", {}).get("version"),
            "source": SPEC_URL,
            "operation_count": len(operations),
        },
        "operations": operations,
    }


def main() -> None:
    src = sys.argv[1] if len(sys.argv) > 1 else SPEC_URL
    out = sys.argv[2] if len(sys.argv) > 2 else DEFAULT_OUT
    digest = build_digest(load_spec(src))
    with open(out, "w") as f:
        json.dump(digest, f, separators=(",", ":"), sort_keys=True)
    print(f"wrote {out}: {len(digest['operations'])} operations")


if __name__ == "__main__":
    main()
