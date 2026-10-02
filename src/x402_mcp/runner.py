"""Generic pieces: tool specs, paid HTTP executor, and the stdio MCP server loop."""

from __future__ import annotations

import argparse
import asyncio
import base64
import json
import logging
import re
import sys
from dataclasses import dataclass, field
from urllib.parse import quote

import httpx
import mcp.types as types
from mcp.server.lowlevel import Server
from mcp.server.stdio import stdio_server

from .config import ConfigError, load_payment_config
from .payment import build_paid_http_client, build_x402_client

logger = logging.getLogger("x402_mcp")

_PATH_PARAM_RE = re.compile(r"\{(\w+)\}")


@dataclass
class ToolSpec:
    name: str
    description: str
    method: str
    path: str
    input_schema: dict
    param_routes: dict[str, str] = field(default_factory=dict)  # name -> path|query|body
    has_body: bool = False

    @property
    def path_params(self) -> list[str]:
        return _PATH_PARAM_RE.findall(self.path)


class PaidApiClient:
    """Issues requests through the x402 payment transport."""

    def __init__(self, x402_client, base_url: str, timeout_s: float):
        self._http, self._helper = build_paid_http_client(x402_client, base_url, timeout_s)
        self.base_url = base_url

    def route_for(self, spec: ToolSpec, arg: str) -> str:
        if arg in spec.param_routes:
            return spec.param_routes[arg]
        if arg in spec.path_params:
            return "path"
        return "body" if spec.has_body else "query"

    async def call(self, spec: ToolSpec, args: dict) -> tuple[int, str, dict | None]:
        path = spec.path
        query: dict = {}
        body: dict = {}
        for key, value in (args or {}).items():
            if value is None:
                continue
            route = self.route_for(spec, key)
            if route == "path":
                path = path.replace("{" + key + "}", quote(str(value), safe=""))
            elif route == "query":
                query[key] = value
            else:
                body[key] = value

        resp = await self._http.request(
            spec.method,
            path,
            params=query or None,
            json=body if spec.has_body else None,
        )
        receipt = self._extract_receipt(resp)
        if receipt is not None:
            logger.info("x402 settled: %s", json.dumps(receipt, default=str)[:500])
        return resp.status_code, self._error_detail(resp) or resp.text, receipt

    def _error_detail(self, resp: httpx.Response) -> str | None:
        """For a still-402 response, surface the payment error from the challenge header."""
        if resp.status_code != 402:
            return None
        for header in ("PAYMENT-REQUIRED", "X-PAYMENT-REQUIRED"):
            raw = resp.headers.get(header)
            if not raw:
                continue
            try:
                challenge = json.loads(base64.b64decode(raw))
            except Exception:
                continue
            error = challenge.get("error")
            if error:
                return json.dumps({"http_status": 402, "payment_error": error})
        return None

    def _extract_receipt(self, resp: httpx.Response) -> dict | None:
        for header in ("PAYMENT-RESPONSE", "X-PAYMENT-RESPONSE"):
            raw = resp.headers.get(header)
            if not raw:
                continue
            try:
                return json.loads(base64.b64decode(raw))
            except Exception:
                return {"header": header, "raw": raw[:200]}
        return None

    async def aclose(self) -> None:
        await self._http.aclose()


def truncate(text: str, max_chars: int) -> str:
    if len(text) <= max_chars:
        return text
    return text[:max_chars] + f"\n...[truncated {len(text) - max_chars} chars]"


def run_stdio_server(
    server_name: str,
    server_version: str,
    tools: list[ToolSpec],
    api: PaidApiClient,
    max_chars: int,
) -> None:
    app = Server(server_name, server_version)
    by_name = {t.name: t for t in tools}

    @app.list_tools()
    async def _list_tools() -> list[types.Tool]:
        return [
            types.Tool(name=t.name, description=t.description, inputSchema=t.input_schema)
            for t in tools
        ]

    @app.call_tool()
    async def _call_tool(name: str, arguments: dict | None) -> types.CallToolResult:
        spec = by_name.get(name)
        if spec is None:
            return _error(f"Unknown tool: {name}")
        try:
            status, text, _receipt = await api.call(spec, arguments or {})
        except httpx.HTTPError as e:
            logger.exception("HTTP error in tool %s", name)
            return _error(f"HTTP error: {type(e).__name__}: {e}")
        except Exception as e:
            logger.exception("error in tool %s", name)
            return _error(f"{type(e).__name__}: {e}")
        if 200 <= status < 300:
            return types.CallToolResult(
                content=[types.TextContent(type="text", text=truncate(text, max_chars))]
            )
        return _error(json.dumps({"http_status": status, "body": truncate(text, max_chars)}))

    def _error(message: str) -> types.CallToolResult:
        return types.CallToolResult(
            content=[types.TextContent(type="text", text=message)], isError=True
        )

    async def _amain() -> None:
        async with stdio_server() as (read, write):
            await app.run(read, write, app.create_initialization_options())

    asyncio.run(_amain())


def build_arg_parser(default_base_url: str, default_timeout: float) -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description=(
            "Stdio MCP server exposing x402-paid APIs as plain tools. "
            "Requires EVM_PRIVATE_KEY (and/or SVM_PRIVATE_KEY) in the environment."
        )
    )
    parser.add_argument("--base-url", default=default_base_url, help="API origin to call")
    parser.add_argument(
        "--timeout", type=float, default=default_timeout, help="HTTP timeout seconds"
    )
    parser.add_argument(
        "--max-price-usd",
        type=float,
        default=None,
        help="Override X402_MAX_PRICE_USD per-payment cap (use X402_MAX_PRICE_USD=none to disable)",
    )
    parser.add_argument(
        "--max-response-chars",
        type=int,
        default=120_000,
        help="Truncate tool results longer than this many characters",
    )
    parser.add_argument("--env-file", default=None, help="Optional .env file to load")
    parser.add_argument(
        "--log-level", default="INFO", help="Logging level (logs go to stderr)"
    )
    parser.add_argument(
        "--list-tools",
        action="store_true",
        help="Print available tools and exit (no wallet needed)",
    )
    return parser


def serve(
    server_name: str,
    server_version: str,
    tools: list[ToolSpec],
    *,
    default_base_url: str,
    default_timeout: float,
    argv: list[str] | None = None,
) -> None:
    parser = build_arg_parser(default_base_url, default_timeout)
    args = parser.parse_args(argv)

    logging.basicConfig(
        level=getattr(logging, args.log_level.upper(), logging.INFO),
        stream=sys.stderr,
        format="%(asctime)s %(levelname)s %(name)s: %(message)s",
    )

    if args.list_tools:
        for t in tools:
            req = t.input_schema.get("required", [])
            print(f"{t.name}\t{t.method} {t.path}\trequired={','.join(req) or '-'}")
            print(f"    {t.description.splitlines()[0]}")
        return

    try:
        cfg = load_payment_config(args.env_file, args.max_price_usd)
    except ConfigError as e:
        parser.exit(2, f"error: {e}\n")
        return

    x402_client = build_x402_client(cfg)
    api = PaidApiClient(x402_client, args.base_url, args.timeout)
    logger.info(
        "serving %s v%s with %d tools against %s",
        server_name,
        server_version,
        len(tools),
        args.base_url,
    )
    run_stdio_server(server_name, server_version, tools, api, args.max_response_chars)
