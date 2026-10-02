import asyncio

import httpx
from x402_mcp.runner import (
    ToolSpec,
    build_arg_parser,
    build_http_app,
    format_tool_listing,
)

TOOLS = [
    ToolSpec(
        name="demo_ping",
        description="ping",
        method="POST",
        path="/ping",
        input_schema={"type": "object", "properties": {}, "additionalProperties": False},
    )
]

HEADERS = {
    "Accept": "application/json, text/event-stream",
    "Content-Type": "application/json",
}
INIT = {
    "jsonrpc": "2.0",
    "id": 1,
    "method": "initialize",
    "params": {
        "protocolVersion": "2025-06-18",
        "capabilities": {},
        "clientInfo": {"name": "smoke", "version": "0"},
    },
}


class FakeApi:
    async def call(self, spec, args):
        return 200, "pong", None


def _drive(coro):
    return asyncio.run(coro)


def test_http_rejects_missing_or_wrong_bearer_token():
    async def main():
        app, manager = build_http_app(
            "t", "0", TOOLS, FakeApi(), 1000, bearer_token="secret"
        )
        async with manager.run():
            async with httpx.AsyncClient(
                transport=httpx.ASGITransport(app=app), base_url="http://test"
            ) as client:
                r = await client.post("/mcp", json=INIT, headers=HEADERS)
                assert r.status_code == 401
                assert r.headers["www-authenticate"] == "Bearer"

                r = await client.post(
                    "/mcp", json=INIT, headers={**HEADERS, "Authorization": "Bearer wrong"}
                )
                assert r.status_code == 401

                r = await client.post(
                    "/mcp", json=INIT, headers={**HEADERS, "Authorization": "Basic abc"}
                )
                assert r.status_code == 401

    _drive(main())


def test_http_roundtrip_with_bearer_token():
    async def main():
        app, manager = build_http_app(
            "t", "0", TOOLS, FakeApi(), 1000, bearer_token="secret"
        )
        auth = {**HEADERS, "Authorization": "Bearer secret"}
        async with manager.run():
            async with httpx.AsyncClient(
                transport=httpx.ASGITransport(app=app), base_url="http://test"
            ) as client:
                r = await client.post("/mcp", json=INIT, headers=auth)
                assert r.status_code == 200
                assert r.json()["result"]["serverInfo"]["name"] == "t"

                r = await client.post(
                    "/mcp",
                    json={"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}},
                    headers=auth,
                )
                assert r.status_code == 200
                assert [t["name"] for t in r.json()["result"]["tools"]] == ["demo_ping"]

                r = await client.post(
                    "/mcp",
                    json={
                        "jsonrpc": "2.0",
                        "id": 3,
                        "method": "tools/call",
                        "params": {"name": "demo_ping", "arguments": {}},
                    },
                    headers=auth,
                )
                assert r.status_code == 200
                assert r.json()["result"]["content"][0]["text"] == "pong"

    _drive(main())


def test_arg_parser_transport_flags():
    parser = build_arg_parser("https://example", 30)
    args = parser.parse_args(
        ["--transport", "http", "--host", "0.0.0.0", "--port", "9000", "--bearer-token", "t"]
    )
    assert args.transport == "http"
    assert args.host == "0.0.0.0"
    assert args.port == 9000
    assert args.bearer_token == "t"

    defaults = parser.parse_args([])
    assert defaults.transport == "stdio"
    assert defaults.host == "127.0.0.1"


def test_http_local_content_tool_and_instructions():
    """local_content tools (e.g. <prefix>_help) never touch the paid API, and
    instructions reaches the initialize handshake."""
    tools = [
        ToolSpec(
            name="demo_help",
            description="Extended documentation.",
            method="GET",
            path="https://example/llms.txt",
            input_schema={"type": "object", "properties": {}},
            local_content=lambda: "guide text",
        ),
    ]

    async def main():
        auth = {**HEADERS, "Authorization": "Bearer secret"}
        app, manager = build_http_app(
            "t",
            "0",
            tools,
            FakeApi(),
            1000,
            bearer_token="secret",
            instructions="Demo guidance.",
        )
        async with manager.run():
            async with httpx.AsyncClient(
                transport=httpx.ASGITransport(app=app), base_url="http://test"
            ) as client:
                r = await client.post("/mcp", json=INIT, headers=auth)
                assert r.status_code == 200
                assert r.json()["result"]["instructions"] == "Demo guidance."

                r = await client.post(
                    "/mcp",
                    json={
                        "jsonrpc": "2.0",
                        "id": 2,
                        "method": "tools/call",
                        "params": {"name": "demo_help", "arguments": {}},
                    },
                    headers=auth,
                )
                assert r.status_code == 200
                assert r.json()["result"]["content"][0]["text"] == "guide text"

                # an unknown tool becomes an isError result
                r = await client.post(
                    "/mcp",
                    json={
                        "jsonrpc": "2.0",
                        "id": 3,
                        "method": "tools/call",
                        "params": {"name": "demo_missing", "arguments": {}},
                    },
                    headers=auth,
                )
                assert r.status_code == 200
                assert r.json()["result"]["isError"] is True

    _drive(main())


def test_format_tool_listing_separates_model_visible_from_internal():
    tools = [
        ToolSpec(
            name="demo_ping",
            description="Ping. Paid per call via x402.",
            method="POST",
            path="/ping",
            input_schema={"type": "object", "properties": {}},
        ),
        ToolSpec(
            name="demo_extract",
            description="Extract a URL.",
            method="POST",
            path="/v1/extract",
            input_schema={
                "type": "object",
                "properties": {
                    "url": {"type": "string", "description": "Page to fetch"},
                    "mode": {"type": "string", "enum": ["fast", "deep"]},
                },
                "required": ["url"],
            },
            param_routes={"url": "body", "mode": "body"},
            has_body=True,
        ),
    ]
    out = format_tool_listing(tools, "x402 Demo", "https://api.example.dev")

    assert out.startswith(
        "2 tools for server 'x402 Demo' — base URL https://api.example.dev"
    )
    # legend explains model-visible vs internal plumbing once, up front
    assert "the model receives" in out
    assert "[path|query|body]" in out
    # endpoint block sits above the description and is internal-only
    assert "  Endpoint:\n    POST /v1/extract\n  Description:" in out
    # verbatim model-visible description, indented under its label to align
    # with the Arguments list
    assert "  Description:" in out
    assert "\n    Ping. Paid per call via x402." in out
    # arguments: name, required star, type, enum, internal route, description
    assert "url*  string [body] Page to fetch" in out
    assert "mode  string (one of: fast|deep) [body]" in out
    # zero-argument tools say so instead of required=-
    assert "  Arguments: none" in out
    # blank-line separator between tools; names render as call signatures
    assert "demo_ping():" in out
    assert "\n\ndemo_extract():" in out
