"""Generic pieces: tool specs, paid HTTP executor, and the stdio/HTTP MCP server loops."""

from __future__ import annotations

import argparse
import asyncio
import base64
import contextlib
import json
import logging
import os
import re
import secrets
import sys
from collections.abc import Callable
from dataclasses import dataclass, field
from urllib.parse import quote

import httpx
import mcp.types as types
import uvicorn
from mcp.server.lowlevel import Server
from mcp.server.stdio import stdio_server
from mcp.server.streamable_http_manager import StreamableHTTPSessionManager
from starlette.applications import Starlette
from starlette.responses import JSONResponse
from starlette.routing import Route

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
    # When set, the tool is served locally (no paid API request): calling it
    # returns local_content()'s text. Used for e.g. <prefix>_help guidance.
    local_content: Callable[[], str] | None = None

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


def build_mcp_server(
    server_name: str,
    server_version: str,
    tools: list[ToolSpec],
    api: PaidApiClient,
    max_chars: int,
    instructions: str | None = None,
) -> Server:
    by_name = {t.name: t for t in tools}

    app = Server(server_name, server_version, instructions=instructions)

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
        if spec.local_content is not None:
            # Served locally, no paid API request (e.g. <prefix>_help). A
            # raised error becomes an isError result; successful fetches are
            # cached inside local_content itself.
            try:
                text = spec.local_content()
            except Exception as e:
                logger.exception("error in tool %s", name)
                return _error(f"{type(e).__name__}: {e}")
            return types.CallToolResult(
                content=[types.TextContent(type="text", text=truncate(text, max_chars))]
            )
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

    return app


def run_stdio_server(
    server_name: str,
    server_version: str,
    tools: list[ToolSpec],
    api: PaidApiClient,
    max_chars: int,
    instructions: str | None = None,
) -> None:
    app = build_mcp_server(server_name, server_version, tools, api, max_chars, instructions)

    async def _amain() -> None:
        async with stdio_server() as (read, write):
            await app.run(read, write, app.create_initialization_options())

    asyncio.run(_amain())


_MCP_HTTP_PATH = "/mcp"


class _BearerGate:
    """ASGI wrapper: require a bearer token, then delegate to the session manager."""

    def __init__(self, manager: StreamableHTTPSessionManager, bearer_token: str):
        self._manager = manager
        self._token = bearer_token

    async def __call__(self, scope, receive, send) -> None:
        if not self._authorized(scope):
            await JSONResponse(
                {"error": "unauthorized"},
                status_code=401,
                headers={"WWW-Authenticate": "Bearer"},
            )(scope, receive, send)
            return
        await self._manager.handle_request(scope, receive, send)

    def _authorized(self, scope) -> bool:
        headers = {
            k.decode("latin-1").lower(): v.decode("latin-1")
            for k, v in (scope.get("headers") or [])
        }
        scheme, _, credential = headers.get("authorization", "").partition(" ")
        if scheme.lower() != "bearer":
            return False
        return secrets.compare_digest(credential.strip(), self._token)


def build_http_app(
    server_name: str,
    server_version: str,
    tools: list[ToolSpec],
    api: PaidApiClient,
    max_chars: int,
    *,
    bearer_token: str,
    instructions: str | None = None,
) -> tuple[Starlette, StreamableHTTPSessionManager]:
    """Return (asgi app, session manager) serving Streamable HTTP at /mcp.

    Stateless + JSON: every POST is one self-contained JSON-RPC exchange, no
    Mcp-Session-Id, no sticky sessions. Callers needing server->client streams
    (progress notifications) would set json_response=False instead.
    """
    mcp_server = build_mcp_server(
        server_name, server_version, tools, api, max_chars, instructions
    )
    manager = StreamableHTTPSessionManager(
        app=mcp_server, json_response=True, stateless=True
    )

    @contextlib.asynccontextmanager
    async def _lifespan(app: Starlette):
        async with manager.run():
            yield

    asgi = Starlette(
        lifespan=_lifespan,
        routes=[
            Route(
                _MCP_HTTP_PATH,
                _BearerGate(manager, bearer_token),
                methods=["POST", "GET", "DELETE"],
            )
        ],
    )
    return asgi, manager


def run_http_server(
    server_name: str,
    server_version: str,
    tools: list[ToolSpec],
    api: PaidApiClient,
    max_chars: int,
    *,
    host: str,
    port: int,
    bearer_token: str,
    instructions: str | None = None,
) -> None:
    asgi, _manager = build_http_app(
        server_name,
        server_version,
        tools,
        api,
        max_chars,
        bearer_token=bearer_token,
        instructions=instructions,
    )
    logger.info(
        "serving streamable HTTP (stateless, JSON) at http://%s:%d%s with bearer auth",
        host,
        port,
        _MCP_HTTP_PATH,
    )
    # log_config=None: uvicorn loggers propagate to the root handler configured
    # in serve(); stdout stays free of MCP traffic on this transport anyway.
    uvicorn.run(asgi, host=host, port=port, log_config=None)


def build_arg_parser(default_base_url: str, default_timeout: float) -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description=(
            "MCP server (stdio or streamable HTTP) exposing x402-paid APIs as "
            "plain tools. Requires EVM_PRIVATE_KEY (and/or SVM_PRIVATE_KEY) in "
            "the environment."
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
    parser.add_argument(
        "--transport",
        choices=("stdio", "http"),
        default="stdio",
        help="stdio (default) or streamable HTTP at /mcp",
    )
    parser.add_argument("--host", default="127.0.0.1", help="HTTP bind address")
    parser.add_argument("--port", type=int, default=8000, help="HTTP bind port")
    parser.add_argument(
        "--bearer-token",
        default=None,
        help=(
            "Bearer token HTTP clients must present (env: X402_MCP_BEARER_TOKEN). "
            "Prefer the env var: CLI args are visible in the process list."
        ),
    )
    return parser


_LIST_RULE = "=" * 78
_ENUM_SHOWN_MAX = 6
_ARG_NAME_WIDTH_MAX = 24


def _arg_route(spec: ToolSpec, arg: str) -> str:
    """Where this server places `arg` in the paid HTTP request (mirrors
    PaidApiClient.route_for; display only — the model never sees this)."""
    route = spec.param_routes.get(arg)
    if route:
        return route
    if arg in spec.path_params:
        return "path"
    return "body" if spec.has_body else "query"


def _arg_type_text(prop: dict) -> str:
    ptype = str(prop.get("type") or "any")
    enum = prop.get("enum")
    if isinstance(enum, list) and enum:
        values = [str(v) for v in enum[:_ENUM_SHOWN_MAX]]
        if len(enum) > _ENUM_SHOWN_MAX:
            values.append("…")
        ptype += f" (one of: {'|'.join(values)})"
    return ptype


def format_tool_listing(
    tools: list[ToolSpec], server_name: str, base_url: str | None
) -> str:
    """Human-readable --list-tools output.

    Separates what the model actually receives (tool name, the verbatim
    description, and the argument properties as a JSON schema) from this
    server's internal request plumbing (HTTP method/path, argument routing),
    and surfaces per-argument descriptions and enums.
    """
    base = f" — base URL {base_url}" if base_url else ""
    lines = [
        f"{len(tools)} tools for server {server_name!r}{base}",
        "",
        'Per tool, the model receives: the name, the full "Description:" text, and',
        "the argument list (name, type, enum, description) as a JSON schema. A",
        "trailing * marks a required argument. The Endpoint block and the",
        "[path|query|body] tag show how this server builds the paid HTTP request",
        "— internal plumbing the model never sees.",
        _LIST_RULE,
    ]
    for i, tool in enumerate(tools):
        if i:
            lines.append("")
        lines.append(f"{tool.name}():")
        lines.append("  Endpoint:")
        lines.append(f"    {tool.method} {tool.path}")
        lines.append("  Description:")
        lines.append(f"    {' '.join(tool.description.split())}")
        props = tool.input_schema.get("properties") or {}
        if not isinstance(props, dict) or not props:
            lines.append("  Arguments: none")
            continue
        required = set(tool.input_schema.get("required") or [])
        lines.append("  Arguments:")
        width = min(max(len(n) for n in props), _ARG_NAME_WIDTH_MAX)
        for pname, prop in props.items():
            prop = prop if isinstance(prop, dict) else {}
            star = "*" if pname in required else ""
            cells = [f"    {pname}{star}".ljust(4 + width + 1), _arg_type_text(prop)]
            cells.append(f"[{_arg_route(tool, pname)}]")
            desc = prop.get("description")
            if isinstance(desc, str) and desc.strip():
                cells.append(" ".join(desc.split()))
            lines.append(" ".join(cells).rstrip())
    return "\n".join(lines)


def serve(
    server_name: str,
    server_version: str,
    tools: list[ToolSpec],
    *,
    default_base_url: str,
    default_timeout: float,
    argv: list[str] | None = None,
    instructions: str | None = None,
) -> None:
    parser = build_arg_parser(default_base_url, default_timeout)
    args = parser.parse_args(argv)

    logging.basicConfig(
        level=getattr(logging, args.log_level.upper(), logging.INFO),
        stream=sys.stderr,
        format="%(asctime)s %(levelname)s %(name)s: %(message)s",
    )

    if args.list_tools:
        print(format_tool_listing(tools, server_name, args.base_url or default_base_url))
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
    if args.transport == "http":
        # load_payment_config has already applied the env file, so the token
        # can live in .env next to the wallet keys.
        token = args.bearer_token or os.getenv("X402_MCP_BEARER_TOKEN")
        if not token:
            parser.exit(
                2,
                "error: --transport http requires --bearer-token or "
                "X402_MCP_BEARER_TOKEN\n",
            )
            return
        run_http_server(
            server_name,
            server_version,
            tools,
            api,
            args.max_response_chars,
            host=args.host,
            port=args.port,
            bearer_token=token,
            instructions=instructions,
        )
    else:
        run_stdio_server(
            server_name, server_version, tools, api, args.max_response_chars, instructions
        )
