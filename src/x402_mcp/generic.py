"""Turn *any* x402 API with an OpenAPI spec into MCP tools.

Configured by a JSON conf file and/or X402_MCP_GENERIC_* env vars (CLI flags
win over env over conf over defaults). Price discovery is done once at
startup by probing selected endpoints *unpaid* — a plain GET expecting a 402
challenge, never a signed request — and cached for the process lifetime, so
repeated tools/list calls cost zero requests.

    x402-mcp-generic --spec https://vendor.dev/openapi.json --include /v1/widgets
    x402-mcp-generic --config vendor.json

See docs/plans/GENERIC_SERVER.md and the README for the conf schema and the
probe semantics (GET-only by default, templated paths skipped, one probe per
endpoint per process).
"""

from __future__ import annotations

import argparse
import base64
import contextlib
import json
import logging
import os
import re
import sys
import time
from collections import Counter
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass, field, fields
from urllib.parse import urlsplit
from urllib.request import Request, urlopen

import httpx
from dotenv import load_dotenv

from .config import ConfigError, USDC_DECIMALS
from .digest import build_operations, load_spec, sanitize_schema, slug_suffix, slugify
from .runner import ToolSpec, serve

logger = logging.getLogger("x402_mcp.generic")

ENV_PREFIX = "X402_MCP_GENERIC_"

_HELP_FETCH_TIMEOUT = 15  # seconds; guidance fetches are small static files

PRICING_LINE_DEFAULT = (
    "Paid per call via x402 (price set by the API; per-payment cap via "
    "X402_MAX_PRICE_USD)."
)

# Assets rendered as USDC dollars for display. Anything else is shown as an
# atomic amount plus the raw asset string — never assume 6 decimals for an
# arbitrary token.
USDC_ASSETS = frozenset(
    {
        "usdc",
        "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913",  # USDC on Base (eip155:8453)
    }
)

# Vendor pricing extensions denominated in these currencies render as dollar
# amounts; anything else keeps its raw currency label (never assume a rate).
_USD_CURRENCIES = frozenset({"USD", "USDC"})


@dataclass(frozen=True)
class GenericConfig:
    """Resolved generic-launcher configuration (one field per conf key)."""

    spec: str
    name: str | None = None
    base_url: str | None = None
    prefix: str | None = None
    include: tuple[str, ...] = ()
    exclude: tuple[str, ...] = ()
    # OpenAPI-tag selection (the vendor's own capability grouping). Ops whose
    # tags intersect exclude_tags are dropped, then ops whose tags do not
    # intersect tags are dropped — composed with the path include/exclude
    # filters. Ops without tags only survive when no tag filter is set.
    tags: tuple[str, ...] = ()
    exclude_tags: tuple[str, ...] = ()
    pricing_key: str | None = None
    probe_pricing: bool = True
    probe_ttl_seconds: float = 3600.0
    probe_concurrency: int = 4
    probe_timeout: float = 5.0
    probe_max_endpoints: int = 200
    probe_methods: tuple[str, ...] = ("GET",)
    overrides: dict = field(default_factory=dict)
    # conf-only: force this value onto every flattened inputSchema's
    # additionalProperties (None = leave the spec's value untouched)
    additional_properties: bool | None = None
    # Short server-level guidance for the MCP initialize handshake's
    # instructions field (authored per conf; clients may or may not surface it)
    instructions_text: str | None = None
    # Extended vendor documentation (llms.txt or similar) served lazily via a
    # <prefix>_help() tool: fetched on first call, cached for the process
    help_url: str | None = None
    # Optional bound on the assembled tool description. None (the default) =
    # never truncate: vendor text reaches the model whole. Set it only
    # deliberately.
    max_description_chars: int | None = None


# --------------------------------------------------------------------------
# Configuration resolution: CLI flag > env var > conf file key > default
# --------------------------------------------------------------------------


def _env(name: str) -> str | None:
    value = os.getenv(ENV_PREFIX + name)
    return value if value else None


def _cast_env(value, env_name: str, cast):
    try:
        return cast(value)
    except (TypeError, ValueError) as e:
        raise ConfigError(f"invalid value for {ENV_PREFIX}{env_name}: {value!r}") from e


def _env_bool(raw: str) -> bool:
    low = raw.strip().lower()
    if low in ("1", "true", "yes", "on"):
        return True
    if low in ("0", "false", "no", "off"):
        return False
    raise ConfigError(f"invalid boolean for {ENV_PREFIX}PROBE: {raw!r}")


def _split_csv(raw: str) -> tuple[str, ...]:
    return tuple(t.strip() for t in raw.split(",") if t.strip())


def _resolve(cli_value, env_name: str, conf_value, default, cast):
    if cli_value is not None:
        return cli_value
    env = _env(env_name) if env_name else None
    if env is not None:
        return _cast_env(env, env_name, cast)
    if conf_value is not None:
        return conf_value
    return default


def _resolve_list(cli_value, env_name: str, conf_value) -> tuple[str, ...]:
    if cli_value is not None:
        return _split_csv(cli_value)
    env = _env(env_name)
    if env is not None:
        return _split_csv(env)
    if conf_value is not None:
        if not isinstance(conf_value, list) or not all(
            isinstance(x, str) for x in conf_value
        ):
            raise ConfigError(f"config '{env_name.lower()}' must be a list of strings")
        return tuple(conf_value)
    return ()


def _load_conf(path: str) -> dict:
    try:
        with open(path) as f:
            conf = json.load(f)
    except OSError as e:
        raise ConfigError(f"cannot read config file {path!r}: {e}") from e
    except json.JSONDecodeError as e:
        raise ConfigError(f"config file {path!r} is not valid JSON: {e}") from e
    if not isinstance(conf, dict):
        raise ConfigError(f"config file {path!r} must contain a JSON object")
    known = {f.name for f in fields(GenericConfig)}
    unknown = sorted(set(conf) - known)
    if unknown:
        raise ConfigError(
            f"unknown config keys in {path!r}: {', '.join(unknown)} "
            f"(known: {', '.join(sorted(known))})"
        )
    return conf


def resolve_config(cli: argparse.Namespace) -> GenericConfig:
    """Resolve every field: CLI flag > env var > conf file key > default."""
    conf_path = cli.config or _env("CONFIG")
    conf = _load_conf(conf_path) if conf_path else {}

    spec = _resolve(cli.spec, "SPEC", conf.get("spec"), None, str)
    if not spec:
        raise ConfigError(
            "no spec configured — set --spec, X402_MCP_GENERIC_SPEC, or the "
            "config key 'spec' (URL or path to an OpenAPI JSON spec or a "
            "pre-built digest)"
        )

    overrides = conf.get("overrides")
    if overrides is None:
        overrides = {}
    if not isinstance(overrides, dict) or not all(
        isinstance(v, dict) for v in overrides.values()
    ):
        raise ConfigError("config 'overrides' must be an object of tool-name -> object")

    probe_concurrency = conf.get("probe_concurrency", 4)
    if not isinstance(probe_concurrency, int) or probe_concurrency < 1:
        raise ConfigError("config 'probe_concurrency' must be a positive integer")
    probe_timeout = conf.get("probe_timeout", 5.0)
    if not isinstance(probe_timeout, (int, float)) or probe_timeout <= 0:
        raise ConfigError("config 'probe_timeout' must be a positive number")
    probe_methods = conf.get("probe_methods")
    if probe_methods is None:
        probe_methods = ("GET",)
    if (
        not isinstance(probe_methods, (list, tuple))
        or not probe_methods
        or not all(isinstance(m, str) for m in probe_methods)
    ):
        raise ConfigError("config 'probe_methods' must be a non-empty list of strings")
    additional_properties = conf.get("additional_properties")
    if additional_properties is not None and not isinstance(additional_properties, bool):
        raise ConfigError("config 'additional_properties' must be a boolean")

    instructions_text = _resolve(
        None, "INSTRUCTIONS_TEXT", conf.get("instructions_text"), None, str
    )
    help_url = _resolve(None, "HELP_URL", conf.get("help_url"), None, str)
    max_description_chars = _resolve(
        None, "MAX_DESCRIPTION_CHARS", conf.get("max_description_chars"), None, int
    )
    if max_description_chars is not None and (
        isinstance(max_description_chars, bool) or max_description_chars <= 0
    ):
        raise ConfigError("max_description_chars must be a positive integer")

    probe_pricing = _resolve(cli.probe, "PROBE", conf.get("probe_pricing"), True, _env_bool)
    probe_ttl_seconds = _resolve(
        cli.probe_ttl, "PROBE_TTL", conf.get("probe_ttl_seconds"), 3600.0, float
    )
    if probe_ttl_seconds <= 0:
        raise ConfigError("probe_ttl_seconds must be positive")
    probe_max_endpoints = _resolve(
        cli.probe_max_endpoints,
        None,
        conf.get("probe_max_endpoints"),
        200,
        int,
    )

    return GenericConfig(
        spec=spec,
        name=_resolve(cli.name, "NAME", conf.get("name"), None, str),
        # No CLI flag here on purpose: --base-url stays owned by serve()'s
        # parser; main() pre-scans it and overrides whatever this resolves.
        base_url=_resolve(None, "BASE_URL", conf.get("base_url"), None, str),
        prefix=_resolve(cli.prefix, "PREFIX", conf.get("prefix"), None, str),
        include=_resolve_list(cli.include, "INCLUDE", conf.get("include")),
        exclude=_resolve_list(cli.exclude, "EXCLUDE", conf.get("exclude")),
        tags=_resolve_list(cli.tags, "TAGS", conf.get("tags")),
        exclude_tags=_resolve_list(cli.exclude_tags, "EXCLUDE_TAGS", conf.get("exclude_tags")),
        pricing_key=_resolve(cli.pricing_key, "PRICING_KEY", conf.get("pricing_key"), None, str),
        probe_pricing=probe_pricing,
        probe_ttl_seconds=float(probe_ttl_seconds),
        probe_concurrency=probe_concurrency,
        probe_timeout=float(probe_timeout),
        probe_max_endpoints=probe_max_endpoints,
        probe_methods=tuple(m.upper() for m in probe_methods),
        overrides=overrides,
        additional_properties=additional_properties,
        instructions_text=instructions_text,
        help_url=help_url,
        max_description_chars=max_description_chars,
    )


# --------------------------------------------------------------------------
# Spec / digest input
# --------------------------------------------------------------------------


def _server_url(root: dict) -> str | None:
    servers = root.get("servers") or []
    if not servers or not isinstance(servers[0], dict):
        return None
    url = servers[0].get("url")
    if not isinstance(url, str) or not url:
        return None
    if urlsplit(url).scheme not in ("http", "https") or "{" in url:
        return None  # relative or templated server URL is not a usable base
    return url


def load_source(spec_src: str, pricing_key: str | None) -> tuple[list[dict], str | None, str | None]:
    """Return (operations, title, server_url) for a spec URL/path or digest path.

    A pre-built digest is detected by shape: top-level `operations` present
    and `paths` absent. Digests carry no servers block, so `server_url` is
    only populated for raw OpenAPI specs.
    """
    try:
        root = load_spec(spec_src)
    except (OSError, ValueError) as e:
        raise ConfigError(f"cannot load spec {spec_src!r}: {e}") from e
    if not isinstance(root, dict):
        raise ConfigError(f"spec {spec_src!r} is not a JSON object")

    if "operations" in root and "paths" not in root:
        ops = root["operations"]
        meta = root.get("meta")
        title = meta.get("title") if isinstance(meta, dict) else None
        server_url = None
    elif "paths" in root:
        try:
            ops = build_operations(root, pricing_key=pricing_key, include_tags=True)
        except (AttributeError, TypeError) as e:
            raise ConfigError(f"spec {spec_src!r} is not a valid OpenAPI object: {e}") from e
        info = root.get("info") or {}
        title = info.get("title") if isinstance(info, dict) else None
        server_url = _server_url(root)
    else:
        raise ConfigError(
            f"{spec_src!r} is neither an OpenAPI spec (no 'paths') nor an "
            "operations digest (no 'operations')"
        )

    if not isinstance(ops, list):
        raise ConfigError(f"spec {spec_src!r}: 'operations' is not a list")
    for i, op in enumerate(ops):
        if (
            not isinstance(op, dict)
            or not isinstance(op.get("path"), str)
            or not isinstance(op.get("method"), str)
        ):
            raise ConfigError(f"spec {spec_src!r}: operation #{i} has no path/method")
    return ops, title, server_url


# --------------------------------------------------------------------------
# Prefix matching, naming, and tool generation
# --------------------------------------------------------------------------


def match_prefix(path: str, prefixes) -> str | None:
    """Longest prefix p with path == p or path.startswith(p + "/").

    The trailing-slash rule keeps /v1/web from swallowing /v1/webhook-*.
    """
    best = None
    for p in prefixes:
        if path == p or path.startswith(p + "/"):
            if best is None or len(p) > len(best):
                best = p
    return best


def default_prefix(base_url: str) -> str:
    """Tool-name prefix derived from the base_url host.

    Drops a leading api/www label and the TLD label, then slugifies:
    api.socialfetch.dev -> socialfetch, stable-deepline.dev -> stable_deepline,
    localhost -> localhost.
    """
    host = urlsplit(base_url).netloc.rsplit("@", 1)[-1].split(":", 1)[0].lower()
    labels = [l for l in host.split(".") if l]
    if not labels:
        return "api"
    if labels[0] in ("api", "www") and len(labels) > 1:
        labels = labels[1:]
    if len(labels) > 1:
        labels = labels[:-1]
    return slugify(".".join(labels))


def select_ops(cfg: GenericConfig, ops: list[dict]) -> list[tuple[dict, str]]:
    """Filter ops by include/exclude path prefixes and OpenAPI tags; return
    (op, include_anchor) pairs.

    The anchor is the longest matching include prefix (or "" when include is
    empty); tool-name slugs are relative to it.
    """
    tags_keep = set(cfg.tags)
    tags_drop = set(cfg.exclude_tags)
    selected = []
    for op in sorted(ops, key=lambda o: (o["path"], o["method"])):
        path = op["path"]
        if cfg.exclude and match_prefix(path, cfg.exclude) is not None:
            continue
        if cfg.include:
            anchor = match_prefix(path, cfg.include)
            if anchor is None:
                continue
        else:
            anchor = ""
        op_tags = set(op.get("tags") or [])
        if tags_drop and op_tags & tags_drop:
            continue
        if tags_keep and not op_tags & tags_keep:
            continue
        selected.append((op, anchor))
    return selected


def assign_names(prefix: str, selected: list[tuple[dict, str]]) -> list[tuple[dict, str]]:
    """Deterministic tool names: {prefix}_{slug}; colliding multi-method paths
    all get _{method}; a final _{n} backstop guarantees uniqueness."""
    base_counts: dict[str, int] = {}
    staged = []
    for op, anchor in selected:
        base = f"{prefix}_{slug_suffix(anchor, op['path'])}"
        base_counts[base] = base_counts.get(base, 0) + 1
        staged.append((op, base))
    seen: dict[str, int] = {}
    named = []
    for op, base in staged:
        name = base if base_counts[base] == 1 else f"{base}_{op['method'].lower()}"
        n = seen.get(name, 0)
        seen[name] = n + 1
        if n:
            name = f"{name}_{n + 1}"
        named.append((op, name))
    return named


_PRICE_TOKEN_RE = re.compile(r"\$\d[\d,]*(?:\.\d+)?")


def _price_tokens_stated(sentence: str, text: str) -> bool:
    """True when every $ amount `sentence` states already appears in `text`."""
    tokens = _PRICE_TOKEN_RE.findall(sentence)
    return bool(tokens) and all(t in text for t in tokens)


def _tool_from_op(
    name: str,
    op: dict,
    pricing_line: str,
    vendor_priced: bool = False,
    max_desc_chars: int | None = None,
) -> ToolSpec:
    """Flatten an operation into a ToolSpec (mirrors socialfetch._tool_from_op)."""
    properties: dict[str, dict] = {}
    required: list[str] = []
    routes: dict[str, str] = {}

    for p in op.get("params") or []:
        if p.get("in") == "header":
            continue  # agent cannot set headers; x402 replaces auth anyway
        prop_name = p.get("name")
        if not prop_name:
            continue
        prop = dict(p.get("schema") or {"type": "string"})
        if p.get("description") and "description" not in prop:
            prop["description"] = p["description"]
        prop.setdefault("type", "string")
        properties[prop_name] = prop
        routes[prop_name] = "path" if p.get("in") == "path" else "query"
        if p.get("required"):
            required.append(prop_name)

    has_body = op.get("body") is not None
    if has_body:
        body = op["body"]["schema"]
        for k, v in (body.get("properties") or {}).items():
            prop_name = k if k not in properties else f"{k}_body"
            if prop_name in properties:
                raise ConfigError(f"unresolvable body/query collision in {op['path']}: {k}")
            properties[prop_name] = v if isinstance(v, dict) else {"type": "string"}
            routes[prop_name] = "body"
        for k in body.get("required") or []:
            required.append(k if k in properties else f"{k}_body")

    input_schema: dict = {"type": "object", "properties": properties}
    if required:
        input_schema["required"] = sorted(set(required))
    input_schema = sanitize_schema(input_schema)

    # One vendor text, never both: the long `description` supersedes the
    # `summary` stub (pairing them duplicated nearly every tool — vendors
    # either echo the summary inside the description or paraphrase it as a
    # terse stub). Falls back to summary, then METHOD path.
    parts = [
        op.get("description") or op.get("summary") or f"{op['method'].upper()} {op['path']}"
    ]
    # When the vendor's own description already states every price our
    # sentence would add, keep their prose and drop ours. Probe/default
    # lines (vendor_priced=False) are never dropped: a live challenge price
    # outranks vendor prose.
    if not (vendor_priced and _price_tokens_stated(pricing_line, " ".join(parts))):
        parts.append(pricing_line)

    return ToolSpec(
        name=name,
        # Vendor text is never trimmed upstream (build_operations trim=False);
        # max_desc_chars is an explicit operator choice (conf
        # max_description_chars), never a silent default.
        description=(
            " ".join(parts)[:max_desc_chars] if max_desc_chars else " ".join(parts)
        ),
        method=op["method"].upper(),
        path=op["path"],
        input_schema=input_schema,
        param_routes=routes,
        has_body=has_body,
    )


# --------------------------------------------------------------------------
# Price discovery — probe once, cache for the process lifetime
# --------------------------------------------------------------------------


class PricingCache:
    """One-shot unpaid price discovery, cached for the process lifetime.

    warm() issues plain, unsigned GETs (no wallet, no x402 transport) and
    expects a 402 + PAYMENT-REQUIRED challenge on payment-gated routes.
    Entries are keyed by (method, url) and never re-probed while the process
    lives: descriptions are built once from the cache, so repeated
    tools/list calls cost zero requests. Entry expiry (probe_ttl_seconds)
    only bounds how long cached data is trusted on lookup — an expired entry
    reads as absent, it never triggers a new request.
    """

    def __init__(
        self,
        *,
        base_url: str,
        ttl_seconds: float,
        timeout: float,
        concurrency: int,
        methods,
        max_endpoints: int,
    ):
        self._base_url = base_url.rstrip("/")
        self._ttl = float(ttl_seconds)
        self._timeout = float(timeout)
        self._concurrency = max(1, int(concurrency))
        self._methods = {str(m).upper() for m in methods}
        self._max_endpoints = max(0, int(max_endpoints))
        self._entries: dict[tuple[str, str], dict] = {}

    def warm(self, ops: list[dict]) -> int:
        """Probe eligible ops once (sorted, capped); return the probed count."""
        candidates = []
        for op in ops:
            if op["method"].upper() not in self._methods:
                continue
            if "{" in op["path"]:
                continue  # unsubstituted templated path gets 404/422, not a 402
            if op.get("pricing"):
                continue  # vendor spec already documents pricing
            candidates.append(op)
        candidates.sort(key=lambda o: (o["path"], o["method"]))
        candidates = candidates[: self._max_endpoints]
        if not candidates:
            return 0
        targets = [(op["method"].upper(), self._base_url + op["path"]) for op in candidates]
        with ThreadPoolExecutor(max_workers=self._concurrency) as pool:
            entries = list(pool.map(self._probe, targets))
        for target, entry in zip(targets, entries):
            self._entries[target] = entry
        logger.info(
            "pricing probe: %d endpoint(s) probed once and cached; "
            "no re-probes while this process lives",
            len(targets),
        )
        return len(targets)

    def get(self, method: str, path: str) -> dict | None:
        entry = self._entries.get((method.upper(), self._base_url + path))
        if entry is None or time.time() >= entry["expires_at"]:
            return None
        return entry

    def _probe(self, target: tuple[str, str]) -> dict:
        method, url = target
        entry = {
            "status": "unreachable",
            "challenge": None,
            "expires_at": time.time() + self._ttl,
        }
        try:
            with httpx.Client(
                timeout=self._timeout,
                follow_redirects=False,
                headers={
                    "User-Agent": "x402-mcp-generic-probe/0.1",
                    "Accept": "application/json",
                },
            ) as client:
                resp = client.request(method, url)
        except Exception as e:  # noqa: BLE001 - a probe must never break boot
            logger.info("pricing probe %s %s: unreachable (%s)", method, url, type(e).__name__)
            return entry
        if resp.status_code == 402:
            challenge = _decode_challenge(resp)
            if challenge is not None:
                entry["status"] = "challenge"
                entry["challenge"] = challenge
            else:
                entry["status"] = "http_402"
        elif 200 <= resp.status_code < 300:
            entry["status"] = "unpaid_2xx"  # reachable but not payment-gated
        else:
            entry["status"] = f"http_{resp.status_code}"
        logger.info("pricing probe %s %s: %s", method, url, entry["status"])
        return entry


def _decode_challenge(resp: httpx.Response) -> dict | None:
    """Extract accepts[0] from the base64 PAYMENT-REQUIRED challenge header."""
    for header in ("PAYMENT-REQUIRED", "X-PAYMENT-REQUIRED"):
        raw = resp.headers.get(header)
        if not raw:
            continue
        try:
            data = json.loads(base64.b64decode(raw))
        except Exception:
            continue
        accepts = data.get("accepts") if isinstance(data, dict) else None
        if isinstance(accepts, list) and accepts and isinstance(accepts[0], dict):
            return accepts[0]
        return None
    return None


def _atomic_amount(value) -> int | None:
    if isinstance(value, bool) or value is None:
        return None
    if isinstance(value, int):
        return value
    if isinstance(value, str):
        try:
            return int(value)
        except ValueError:
            return None
    return None


def _is_usdc(asset) -> bool:
    return isinstance(asset, str) and asset.lower() in USDC_ASSETS


def _usd_text(atomic: int) -> str:
    usd = atomic / 10**USDC_DECIMALS
    return "$" + f"{usd:.6f}".rstrip("0").rstrip(".")


def _challenge_line(challenge: dict) -> str | None:
    scheme = str(challenge.get("scheme") or "exact").lower()
    asset = challenge.get("asset")
    network = challenge.get("network")
    # v2 challenges carry `amount`; the plan/older specs say maxAmountRequired.
    atomic = _atomic_amount(challenge.get("maxAmountRequired"))
    if atomic is None:
        atomic = _atomic_amount(challenge.get("amount"))
    if atomic is None or not asset:
        return None
    if _is_usdc(asset):
        amount = _usd_text(atomic)
        tail = f"USDC on {network}" if network else "USDC"
    else:
        amount = f"{atomic} atomic"
        tail = f"asset {asset}" + (f" on {network}" if network else "")
    paren = f"x402 {scheme}, {tail}"
    if scheme == "upto":
        return f"Metered: up to {amount} per call ({paren})."
    return f"Price: {amount} per call ({paren})."


_MONEY_RE = re.compile(r"\d+(?:\.\d+)?")


def _money_text(value) -> str | None:
    """Vendor price as display text ("0.100000" -> "0.10"), or None.

    Amount text stays as close to the vendor's writing as possible (only
    trailing zeros beyond two decimals are trimmed) so rendered amounts
    remain comparable with the vendor's own prose. Floats are rejected:
    their formatting is ambiguous.
    """
    if isinstance(value, bool) or value is None:
        return None
    text = str(value).strip() if isinstance(value, (str, int)) else None
    if text is None or not _MONEY_RE.fullmatch(text):
        return None
    if "." in text:
        whole, _, frac = text.partition(".")
        return f"{whole}.{frac.rstrip('0').ljust(2, '0')}"
    return text


def _vendor_pricing_line(pricing) -> str | None:
    """Render a vendor pricing extension (the pricing_key value) as prose.

    Understands the {"price": {"amount" | "min"+"max", "currency", "mode"}}
    shape used by x-payment-info, plus the bare {"authMode": "free"} marker
    some vendors publish for ungated routes; anything else returns None so
    the caller falls back to dumping the raw extension as JSON. Non-USD
    currencies are shown as "<amount> <currency>" — never converted or
    assumed to be dollars.
    """
    if not isinstance(pricing, dict):
        return None
    if pricing.get("authMode") == "free" and "price" not in pricing:
        return "Free — no payment required (vendor spec)."
    block = pricing.get("price")
    if not isinstance(block, dict):
        block = pricing
    currency = block.get("currency")

    def shown(text: str) -> str:
        if isinstance(currency, str) and currency.upper() in _USD_CURRENCIES:
            return f"${text}"
        return f"{text} {currency}" if isinstance(currency, str) else text

    single = _money_text(block.get("amount"))
    lo = _money_text(block.get("min"))
    hi = _money_text(block.get("max"))
    if single is not None:
        line = f"Price: {shown(single)} per call"
    elif lo is not None and hi is not None:
        line = f"Price: {shown(lo)}\u2013{shown(hi)} per call, scaling with usage"
    else:
        return None
    notes = ["vendor spec"]
    if single is not None and str(block.get("mode") or "").lower() == "dynamic":
        notes.append("dynamic")
    return f"{line} ({', '.join(notes)})."


def _pricing_line(op: dict, cache: PricingCache | None) -> tuple[str, bool]:
    """(description sentence, sentence derived from the vendor extension)."""
    if op.get("pricing"):
        rendered = _vendor_pricing_line(op["pricing"])
        if rendered:
            return rendered, True
        compact = json.dumps(op["pricing"], separators=(",", ":"), sort_keys=True)
        return f"Pricing: {compact} (vendor spec extension).", True
    if cache is not None:
        entry = cache.get(op["method"], op["path"])
        if entry and entry["status"] == "challenge" and entry["challenge"]:
            line = _challenge_line(entry["challenge"])
            if line:
                return line, False
    return PRICING_LINE_DEFAULT, False


def apply_overrides(tools: list[ToolSpec], overrides: dict) -> None:
    """Apply description/param-description patches last (after probe data).

    Unknown tool names are warnings, not errors: specs drift, and a stale
    override should not break boot.
    """
    if not overrides:
        return
    by_name = {t.name: t for t in tools}
    for name, patch in overrides.items():
        tool = by_name.get(name)
        if tool is None:
            logger.warning("override for unknown tool %r ignored", name)
            continue
        for key in patch:
            if key not in ("description", "params"):
                logger.warning("override key %r on tool %r ignored", key, name)
        description = patch.get("description")
        if description is not None and not isinstance(description, str):
            logger.warning("override description on tool %r must be a string", name)
            description = None
        if description is not None:
            tool.description = description
        params = patch.get("params") or {}
        if not isinstance(params, dict):
            logger.warning("override params on tool %r must be an object", name)
            params = {}
        props = tool.input_schema.setdefault("properties", {})
        for pname, pdesc in params.items():
            if pname in props and isinstance(props[pname], dict):
                if not isinstance(pdesc, str):
                    logger.warning("override param %r on tool %r must be a string", pname, name)
                    continue
                props[pname] = {**props[pname], "description": pdesc}
            else:
                logger.warning("override param %r not present on tool %r", pname, name)


def _help_tool(prefix: str, url: str) -> ToolSpec:
    """<prefix>_help(): extended vendor documentation, lazy-fetched and cached.

    The fetch happens on first tool CALL (never at boot or under --list-tools),
    so boot stays wallet/network-free. Successful fetches are cached for the
    process lifetime; failures raise and may succeed on a later call.
    """
    cache: dict[str, str] = {}

    def fetch() -> str:
        if url not in cache:
            logger.info("fetching extended guidance %s", url)
            req = Request(url, headers={"User-Agent": "x402-mcp-generic-help/0.1"})
            with urlopen(req, timeout=_HELP_FETCH_TIMEOUT) as resp:
                cache[url] = resp.read().decode("utf-8", errors="replace")
        return cache[url]

    return ToolSpec(
        name=f"{prefix}_help",
        description=(
            f"Extended documentation for all {prefix}_* tools: API-wide usage "
            f"guidance, pricing notes, and workflows published by the vendor "
            f"(llms.txt). Takes no arguments and returns the full document. "
            f"Call this before other {prefix}_* tools when unsure how to use "
            f"them."
        ),
        method="GET",
        path=url,
        input_schema={"type": "object", "properties": {}},
        local_content=fetch,
    )


def build_tools(
    cfg: GenericConfig,
    ops: list[dict],
    *,
    base_url: str,
    prefix: str,
    probe: bool,
) -> list[ToolSpec]:
    """Filtered, named, pricing-annotated ToolSpecs for the resolved config."""
    selected = select_ops(cfg, ops)
    if not selected:
        raise ConfigError(
            "no operations matched — check include/exclude prefixes against "
            f"the spec ({len(ops)} operations available)"
        )

    cache: PricingCache | None = None
    if probe and cfg.probe_pricing:
        cache = PricingCache(
            base_url=base_url,
            ttl_seconds=cfg.probe_ttl_seconds,
            timeout=cfg.probe_timeout,
            concurrency=cfg.probe_concurrency,
            methods=cfg.probe_methods,
            max_endpoints=cfg.probe_max_endpoints,
        )
        cache.warm([op for op, _ in selected])

    tools = []
    for op, name in assign_names(prefix, selected):
        pricing_line, vendor_priced = _pricing_line(op, cache)
        tool = _tool_from_op(
            name, op, pricing_line, vendor_priced=vendor_priced,
            max_desc_chars=cfg.max_description_chars,
        )
        if cfg.additional_properties is not None:
            tool.input_schema["additionalProperties"] = cfg.additional_properties
        tools.append(tool)
    if cfg.help_url:
        tools.append(_help_tool(prefix, cfg.help_url))
    if cfg.additional_properties is not None:
        # the help tool's zero-arg schema is covered too: a conf that closes
        # its tool schemas means every tool, and OpenAI strict mode rejects
        # any open one
        for tool in tools:
            tool.input_schema["additionalProperties"] = cfg.additional_properties
    apply_overrides(tools, cfg.overrides)
    return tools


# --------------------------------------------------------------------------
# CLI
# --------------------------------------------------------------------------


def _generic_parser() -> argparse.ArgumentParser:
    # add_help=False: --help falls through to serve()'s parser, which also
    # documents the shared flags (--base-url, --transport, ...).
    parser = argparse.ArgumentParser(add_help=False)
    parser.add_argument(
        "--config", default=None, help="JSON conf file; keys match the config fields (README)"
    )
    parser.add_argument(
        "--spec",
        default=None,
        help="OpenAPI spec URL/path, or path to a pre-built digest (required)",
    )
    parser.add_argument("--name", default=None, help="Server display name (default: spec info.title)")
    parser.add_argument(
        "--prefix", default=None, help="Tool-name prefix (default: derived from the host)"
    )
    parser.add_argument(
        "--include",
        default=None,
        help="Comma-separated path prefixes to expose (default: all operations)",
    )
    parser.add_argument(
        "--exclude", default=None, help="Comma-separated path prefixes to drop"
    )
    parser.add_argument(
        "--tags",
        default=None,
        help="Comma-separated OpenAPI tags to keep (default: all operations)",
    )
    parser.add_argument(
        "--exclude-tags", default=None, help="Comma-separated OpenAPI tags to drop"
    )
    parser.add_argument(
        "--list-tags",
        action="store_true",
        help="Print the spec's OpenAPI tags with operation counts and exit",
    )
    parser.add_argument(
        "--pricing-key",
        default=None,
        help="Vendor pricing extension key to read from raw specs (e.g. x-acme-credits)",
    )
    parser.add_argument(
        "--probe",
        action=argparse.BooleanOptionalAction,
        default=None,
        help="Probe prices once at startup via unpaid GETs (default: on)",
    )
    parser.add_argument(
        "--probe-ttl",
        type=float,
        default=None,
        help="Seconds a probed price stays trusted (default: 3600)",
    )
    parser.add_argument(
        "--probe-max-endpoints",
        type=int,
        default=None,
        help="Cap on endpoints probed at startup (default: 200)",
    )
    return parser


def _scan_flag_value(argv: list[str], flag: str) -> str | None:
    for i, arg in enumerate(argv):
        if arg == flag and i + 1 < len(argv):
            return argv[i + 1]
        if arg.startswith(flag + "="):
            return arg[len(flag) + 1 :]
    return None


@contextlib.contextmanager
def _probe_logging():
    """Temporary stderr handler for pre-serve() logs (stdout is the MCP wire).

    Removed in a finally so serve()'s later logging.basicConfig still honours
    --log-level (calling basicConfig here first would make it a no-op).
    """
    handler = logging.StreamHandler(sys.stderr)
    handler.setFormatter(
        logging.Formatter("%(asctime)s %(levelname)s %(name)s: %(message)s")
    )
    log = logging.getLogger("x402_mcp.generic")
    log.addHandler(handler)
    log.setLevel(logging.INFO)
    try:
        yield
    finally:
        log.removeHandler(handler)
        log.setLevel(logging.NOTSET)


def main(argv: list[str] | None = None) -> None:
    argv = list(sys.argv[1:] if argv is None else argv)
    pre_args, rest = _generic_parser().parse_known_args(argv)

    # Generic config resolves before serve() runs load_payment_config (which
    # is what load_dotenv's), so pre-scan --env-file and load it here to let
    # X402_MCP_GENERIC_* live in .env too.
    env_file = _scan_flag_value(argv, "--env-file")
    if env_file:
        load_dotenv(env_file, override=True)
    else:
        load_dotenv()

    # --base-url / --list-tools are owned by serve()'s parser; pre-scan them.
    cli_base_url = _scan_flag_value(rest, "--base-url")
    list_tools = "--list-tools" in rest

    try:
        with _probe_logging():
            cfg = resolve_config(pre_args)
            ops, title, server_url = load_source(cfg.spec, cfg.pricing_key)
            if pre_args.list_tags:
                # inventory only: no wallet, no probing, no tool building
                counts: Counter = Counter()
                for op in ops:
                    for tag in op.get("tags") or ["(untagged)"]:
                        counts[tag] += 1
                print(f"tags for {cfg.spec}:")
                for tag in sorted(counts):
                    print(f"  {tag} ({counts[tag]} op{'s' if counts[tag] != 1 else ''})")
                return
            base_url = cli_base_url or cfg.base_url or server_url
            if not base_url:
                raise ConfigError(
                    "no base_url — set --base-url, X402_MCP_GENERIC_BASE_URL, "
                    "the config key 'base_url', or spec servers[0].url "
                    "(pre-built digests carry no servers block)"
                )
            prefix = cfg.prefix or default_prefix(base_url)
            name = cfg.name or title or prefix
            tools = build_tools(cfg, ops, base_url=base_url, prefix=prefix, probe=not list_tools)
    except ConfigError as e:
        print(f"error: {e}", file=sys.stderr)
        raise SystemExit(2) from None

    serve(
        f"x402 {name}",
        "0.1.0",
        tools,
        default_base_url=base_url,
        default_timeout=60.0,
        argv=rest,
        instructions=cfg.instructions_text,
    )


if __name__ == "__main__":
    main(sys.argv[1:])
