#!/usr/bin/env python3
"""Build a compact operations digest from the Social Fetch OpenAPI spec.

Usage:
    python scripts/build_socialfetch_digest.py [openapi.json|-URL-] [out.json]

Thin wrapper over x402_mcp.digest: strips response schemas (which make the
raw spec ~24MB) and keeps only what is needed to generate MCP tool
definitions: path, method, summary, description, pricing extensions,
parameters, and request body schemas.

The emitted digest must stay byte-identical across refactors (sort_keys,
compact separators, hardcoded meta.source) — it is committed to the repo.
"""

from __future__ import annotations

import json
import sys

from x402_mcp.digest import build_operations, load_spec

PRICING_KEY = "x-socialfetch-credits-pricing"
DEFAULT_SPEC_URL = "https://www.socialfetch.dev/openapi.json"
DEFAULT_OUT = "src/x402_mcp/data/socialfetch_openapi_digest.json"


def main() -> None:
    src = sys.argv[1] if len(sys.argv) > 1 else DEFAULT_SPEC_URL
    out = sys.argv[2] if len(sys.argv) > 2 else DEFAULT_OUT

    spec = load_spec(src)
    # trim=True compacts descriptions for the committed artifact; the generic
    # launcher never trims (vendor instructions must reach the model uncut).
    operations = build_operations(spec, pricing_key=PRICING_KEY, trim=True)

    digest = {
        "meta": {
            "title": spec.get("info", {}).get("title"),
            "version": spec.get("info", {}).get("version"),
            "source": DEFAULT_SPEC_URL,
            "operation_count": len(operations),
        },
        "operations": operations,
    }

    with open(out, "w") as f:
        json.dump(digest, f, separators=(",", ":"), sort_keys=True)
    print(f"wrote {out}: {len(operations)} operations")


if __name__ == "__main__":
    main()
