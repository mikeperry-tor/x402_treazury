"""Shared OpenAPI -> operations-digest helpers.

Used by scripts/build_socialfetch_digest.py (writes the committed Social Fetch
digest) and by the generic launcher (digests any spec in memory). The digest
shape — compact operation entries with minified param/body schemas — is what
`socialfetch.build_tools()` and `generic` consume.
"""

from __future__ import annotations

import json
import re
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

_HTTP_METHODS = ("get", "post", "put", "patch", "delete")

_NONSLUG_RE = re.compile(r"[^0-9a-z]+")


def _numeric_exclusive(key: str, value, schema: dict):
    """Numeric value for an exclusiveMinimum/Maximum key, or None to drop it.

    OpenAPI 3.0 (draft-04) writes these as booleans modifying minimum/maximum;
    JSON Schema 2020-12 — what OpenAI's tool-schema validation enforces —
    requires numbers. `true` converts from the numeric sibling bound; `false`
    is redundant (the plain minimum/maximum already includes the bound);
    numbers pass through; anything else is dropped rather than emitted in a
    form validators reject.
    """
    if isinstance(value, bool):
        if not value:
            return None
        sibling = schema.get("minimum" if key == "exclusiveMinimum" else "maximum")
        if isinstance(sibling, (int, float)) and not isinstance(sibling, bool):
            return sibling
        return None
    if isinstance(value, (int, float)):
        return value
    return None


def sanitize_schema(value):
    """Deep-copy a JSON Schema with boolean exclusiveMinimum/Maximum bounds
    converted to the numeric form OpenAI's tool-schema validation requires
    (draft-04 `exclusiveMinimum: true` + `minimum: 0` becomes
    `exclusiveMinimum: 0`; unconvertible booleans are dropped). Numbers,
    `false`, and every other key pass through unchanged."""
    if isinstance(value, dict):
        out = {}
        for k, v in value.items():
            if k in ("exclusiveMinimum", "exclusiveMaximum"):
                bound = _numeric_exclusive(k, v, value)
                if bound is not None:
                    out[k] = bound
            else:
                out[k] = sanitize_schema(v)
        return out
    if isinstance(value, list):
        return [sanitize_schema(v) for v in value]
    return value


def trim_desc(value):
    if isinstance(value, str) and len(value) > MAX_DESC:
        return value[: MAX_DESC - 1].rstrip() + "…"
    return value


def minify_schema(s, depth: int = 0, trim: bool = False):
    if not isinstance(s, dict):
        return s
    out = {}
    for k in KEEP_SCALAR_KEYS:
        if k in ("exclusiveMinimum", "exclusiveMaximum"):
            if k in s:
                bound = _numeric_exclusive(k, s[k], s)
                if bound is not None:
                    out[k] = bound
        elif k in s:
            out[k] = s[k]
    if "description" in s:
        out["description"] = trim_desc(s["description"]) if trim else s["description"]
    if "title" in s:
        out["title"] = s["title"]
    if depth < 8:
        if "items" in s:
            out["items"] = minify_schema(s["items"], depth + 1, trim=trim)
        if "properties" in s:
            out["properties"] = {
                k: minify_schema(v, depth + 1, trim=trim)
                for k, v in s["properties"].items()
            }
        if "required" in s and isinstance(s["required"], list):
            out["required"] = s["required"]
        ap = s.get("additionalProperties")
        if isinstance(ap, dict):
            out["additionalProperties"] = minify_schema(ap, depth + 1, trim=trim)
        elif isinstance(ap, bool):
            out["additionalProperties"] = ap
        for comb in ("anyOf", "oneOf", "allOf"):
            if comb in s:
                out[comb] = [minify_schema(v, depth + 1, trim=trim) for v in s[comb][:6]]
    return out


def slugify(text: str) -> str:
    """Lowercase and collapse non-alphanumerics to single underscores."""
    slug = _NONSLUG_RE.sub("_", text.lower()).strip("_")
    return slug or "root"


def slug_suffix(prefix: str, path: str) -> str:
    """Slugified portion of `path` after `prefix`, for embedding in tool names."""
    return slugify(path[len(prefix) :].strip("/"))


def load_spec(src: str) -> dict:
    """Load a JSON OpenAPI spec from an http(s) URL or a local file path."""
    if src.startswith("http"):
        req = urllib.request.Request(src, headers={"User-Agent": "x402-mcp-digest/0.1"})
        raw = urllib.request.urlopen(req, timeout=120).read()
        return json.loads(raw)
    with open(src) as f:
        return json.load(f)


def _resolve_local_refs(spec: dict, schema, seen: frozenset = frozenset()):
    """Deep-copy a schema with local `#/…` $ref pointers replaced by their
    in-spec targets (cycle-guarded).

    External/URL refs and dangling pointers are left verbatim — minify_schema
    then reduces them exactly as it did before dereferencing existed, so
    previously-built digests stay byte-identical on regeneration. A ref cycle
    is cut with a bare object (mirrors scripts/build_lonestar_digest.py).
    """
    if not isinstance(schema, dict):
        return schema
    ref = schema.get("$ref")
    if isinstance(ref, str) and ref.startswith("#/"):
        node = spec
        for part in ref[2:].split("/"):
            part = part.replace("~1", "/").replace("~0", "~")
            if not isinstance(node, dict) or part not in node:
                return schema
            node = node[part]
        if ref in seen:
            return {"type": "object"}
        return _resolve_local_refs(spec, node, seen | {ref})
    out = {}
    for k, v in schema.items():
        if isinstance(v, dict):
            out[k] = _resolve_local_refs(spec, v, seen)
        elif isinstance(v, list):
            out[k] = [
                _resolve_local_refs(spec, i, seen) if isinstance(i, (dict, list)) else i
                for i in v
            ]
        else:
            out[k] = v
    return out


def build_operations(
    spec: dict, pricing_key: str | None = None, trim: bool = False,
    include_tags: bool = False,
) -> list[dict]:
    """Flatten an OpenAPI spec into compact operation entries (digest shape).

    Response schemas are dropped; parameters and request bodies are minified.
    `pricing_key` names a vendor pricing extension to copy into each entry's
    `pricing` field; None (the default) leaves `pricing` as None.

    `include_tags` copies each operation's OpenAPI `tags` (strings only) into
    a `tags` field for tag-based selection. It is opt-in so digest-BUILDING
    scripts keep their byte-identical committed artifacts; the generic
    launcher opts in at boot (committed digests simply carry no tags).

    `trim` cuts descriptions at MAX_DESC chars. It is for compact *digest
    artifacts* (the committed socialfetch digest); tool-generation paths must
    leave it False so vendor usage instructions reach the model uncut.
    """
    operations = []
    for path, ops in sorted(spec.get("paths", {}).items()):
        for method, op in ops.items():
            if method not in _HTTP_METHODS:
                continue
            entry = {
                "path": path,
                "method": method,
                "summary": op.get("summary"),
                "description": trim_desc(op.get("description")) if trim else op.get("description"),
                "pricing": op.get(pricing_key) if pricing_key else None,
            }
            if include_tags:
                entry["tags"] = [
                    t for t in op.get("tags") or [] if isinstance(t, str)
                ]
            entry["params"] = []
            for raw_param in op.get("parameters", []):
                # An operation-level parameter entry may itself be a local
                # $ref to a shared components/parameters object (OpenAPI
                # ignores $ref siblings); resolve it so the shared param
                # reaches flattened tool schemas. External/unresolvable refs
                # keep no name and are skipped. Path-item-level parameters
                # are a separate OpenAPI feature and remain out of scope.
                p = raw_param
                if (
                    isinstance(p, dict)
                    and isinstance(p.get("$ref"), str)
                    and p["$ref"].startswith("#/")
                ):
                    p = _resolve_local_refs(spec, p)
                if not isinstance(p, dict) or not p.get("name"):
                    continue
                schema = minify_schema(
                    _resolve_local_refs(spec, p.get("schema", {})), trim=trim
                )
                raw_desc = p.get("description") or schema.get("description")
                desc = trim_desc(raw_desc) if trim else raw_desc
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
                    "schema": minify_schema(
                        _resolve_local_refs(spec, content.get("schema", {})), trim=trim
                    ),
                }
            else:
                entry["body"] = None
            operations.append(entry)
    return operations
