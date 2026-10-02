"""Social Fetch (api.socialfetch.dev) as MCP tools, pay-per-call with x402.

Tools are generated from a digest of the public OpenAPI spec, filtered to the
platforms you select on the command line. Each platform is a separate
invocation/subset, e.g.:

    x402-mcp-socialfetch --platforms tiktok,twitter,youtube
    x402-mcp-socialfetch --platforms linkedin-v2
    x402-mcp-socialfetch --list-platforms

Pricing is metered in credits at $0.014/credit (see tool descriptions).
Metered search endpoints use the x402 `upto` scheme: the wallet must have a
one-time USDC.approve(Permit2) on Base for those to settle.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from importlib.resources import files

from .runner import ToolSpec, serve

BASE_URL = "https://api.socialfetch.dev"

CREDIT_USD = 0.014

# platform selector token -> list of (openapi path prefix, tool name prefix)
PLATFORMS: dict[str, list[tuple[str, str]]] = {
    "amazon": [("/v1/amazon", "amazon")],
    "apple-music": [("/v1/apple-music", "apple_music")],
    "bluesky": [("/v1/bluesky", "bluesky")],
    "facebook": [("/v1/facebook", "facebook")],
    "github": [("/v1/github", "github")],
    "google": [("/v1/google", "google")],
    "hacker-news": [("/v1/hackernews", "hackernews")],
    "instagram": [("/v1/instagram", "instagram")],
    "linkedin": [("/v1/linkedin", "linkedin_v1"), ("/v2/linkedin", "linkedin_v2")],
    "linkedin-v1": [("/v1/linkedin", "linkedin_v1")],
    "linkedin-v2": [("/v2/linkedin", "linkedin_v2")],
    "linktree": [("/v1/linktree", "linktree")],
    "pinterest": [("/v1/pinterest", "pinterest")],
    "reddit": [("/v1/reddit", "reddit")],
    "rumble": [("/v1/rumble", "rumble")],
    "soundcloud": [("/v1/soundcloud", "soundcloud")],
    "spotify": [("/v1/spotify", "spotify")],
    "telegram": [("/v1/telegram", "telegram")],
    "threads": [("/v1/threads", "threads")],
    "tiktok": [("/v1/tiktok", "tiktok")],
    "truthsocial": [("/v1/truthsocial", "truthsocial")],
    "twitch": [("/v1/twitch", "twitch")],
    "twitter": [("/v1/twitter", "twitter")],
    "web": [("/v1/web", "web")],
    "youtube": [("/v1/youtube", "youtube")],
}

ALIASES = {
    "hn": "hacker-news",
    "hackernews": "hacker-news",
    "x": "twitter",
    "linkedin2": "linkedin-v2",
    "applemusic": "apple-music",
    "reddit-com": "reddit",
}

# Headline platforms from https://www.socialfetch.dev/docs/api sections
MAIN = ["tiktok", "instagram", "twitter", "youtube", "facebook", "linkedin", "reddit"]

# Not available over the x402 rail (need an API key) or interactive-account only.
EXCLUDED_PREFIXES = ("/v1/whoami", "/v1/balance", "/v1/ask", "/v1/monitors", "/v1/webhook")

_NONSLUG_RE = re.compile(r"[^0-9a-z]+")


def load_digest() -> dict:
    raw = files("x402_mcp.data").joinpath("socialfetch_openapi_digest.json").read_text()
    return json.loads(raw)


def resolve_selector(token: str) -> list[tuple[str, str]]:
    key = token.strip().lower().replace("_", "-")
    key = ALIASES.get(key, key)
    if key not in PLATFORMS:
        raise SystemExit(
            f"unknown platform {token!r}. Available: {', '.join(sorted(PLATFORMS))} "
            "(also: 'main', 'all')"
        )
    return PLATFORMS[key]


def _slug(prefix: str, path: str, method: str) -> str:
    rest = path[len(prefix):].strip("/")
    slug = _NONSLUG_RE.sub("_", rest.lower()).strip("_")
    if not slug:
        slug = "root"
    return slug


def build_tools(platforms: list[str]) -> list[ToolSpec]:
    digest = load_digest()
    selected: list[tuple[str, str]] = []
    for token in platforms:
        for sel in resolve_selector(token):
            if sel not in selected:
                selected.append(sel)

    tools: list[ToolSpec] = []
    seen: set[str] = set()
    for op in digest["operations"]:
        path = op["path"]
        if any(path.startswith(x) for x in EXCLUDED_PREFIXES):
            continue
        for prefix, tprefix in selected:
            if path != prefix and not path.startswith(prefix + "/"):
                continue
            name = f"{tprefix}_{_slug(prefix, path, op['method'])}"
            if name in seen:
                raise RuntimeError(f"duplicate tool name {name}")
            seen.add(name)
            tools.append(_tool_from_op(name, op))
            break
    return tools


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
        if op["body"].get("required") and not body.get("required"):
            pass  # body itself required, no individual fields marked

    input_schema: dict = {"type": "object", "properties": properties}
    if required:
        input_schema["required"] = sorted(set(required))

    desc_parts = [op.get("summary") or f"{op['method'].upper()} {op['path']}"]
    if op.get("description") and op["description"] != op.get("summary"):
        desc_parts.append(op["description"])
    if op.get("pricing"):
        desc_parts.append(f"Pricing: {op['pricing']} (~${CREDIT_USD}/credit via x402 USDC on Base).")
    desc_parts.append(f"Endpoint: {op['method'].upper()} {path_of(op)}.")

    return ToolSpec(
        name=name,
        description=" ".join(desc_parts)[:1024],
        method=op["method"].upper(),
        path=path_of(op),
        input_schema=input_schema,
        param_routes=routes,
        has_body=has_body,
    )


def path_of(op: dict) -> str:
    return op["path"]


def _platform_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(add_help=False)
    parser.add_argument(
        "--platforms",
        default=None,
        help="Comma-separated platform subset (repeatable). See --list-platforms. "
        "Use 'main' or 'all'.",
    )
    parser.add_argument("--list-platforms", action="store_true", help="List platform selectors and exit")
    return parser


def main(argv: list[str] | None = None) -> None:
    argv = list(sys.argv[1:] if argv is None else argv)
    pre_args, rest = _platform_parser().parse_known_args(argv)

    if pre_args.list_platforms:
        print("selectors (use with --platforms, comma-separated):")
        print("  main          " + " ".join(MAIN))
        print("  all           every selector below")
        for key in sorted(PLATFORMS):
            print(f"  {key:14} -> {', '.join(p for p, _ in PLATFORMS[key])}")
        print("aliases: " + ", ".join(f"{k}={v}" for k, v in sorted(ALIASES.items())))
        return

    if not pre_args.platforms:
        _platform_parser().error("--platforms is required (see --list-platforms)")

    tokens = [t.strip() for t in pre_args.platforms.split(",") if t.strip()]
    expanded: list[str] = []
    for t in tokens:
        if t.lower() == "all":
            expanded.extend(PLATFORMS)
        elif t.lower() == "main":
            expanded.extend(MAIN)
        else:
            expanded.append(t)

    try:
        tools = build_tools(expanded)
    except SystemExit:
        raise
    if not tools:
        _platform_parser().error(f"--platforms {pre_args.platforms!r} matched no endpoints")

    serve(
        f"x402 Social Fetch [{','.join(dict.fromkeys(expanded))}]",
        "0.1.0",
        tools,
        default_base_url=BASE_URL,
        default_timeout=90.0,
        argv=rest,
    )


if __name__ == "__main__":
    main(sys.argv[1:])
