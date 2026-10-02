#!/usr/bin/env python3
"""Spawn an x402 MCP server over stdio and exercise initialize/list_tools/call_tool.

Usage:
    python scripts/smoke_stdio.py [--tool NAME] [--args JSON] -- <server command...>
"""

from __future__ import annotations

import argparse
import asyncio
import json
import os
import sys

from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client


async def run(cmd: list[str], tool: str | None, args: dict, extra_env: dict[str, str]) -> None:
    env = dict(os.environ)
    env.update(extra_env)
    params = StdioServerParameters(command=cmd[0], args=cmd[1:], env=env)
    async with stdio_client(params) as (read, write):
        async with ClientSession(read, write) as session:
            init = await session.initialize()
            print(f"server: {init.serverInfo.name} v{init.serverInfo.version}")
            tools = await session.list_tools()
            names = [t.name for t in tools.tools]
            print(f"tools ({len(names)}): {', '.join(names[:12])}{' ...' if len(names) > 12 else ''}")
            if tool:
                match = next((t for t in tools.tools if t.name == tool), None)
                if match:
                    print(f"schema {tool}: {json.dumps(match.inputSchema)[:600]}")
                result = await session.call_tool(tool, args)
                is_err = getattr(result, "isError", False)
                print(f"call_tool {tool} isError={is_err}")
                for block in result.content:
                    text = getattr(block, "text", str(block))
                    print(text[:1200])


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--tool", default=None)
    parser.add_argument("--args", default="{}")
    parser.add_argument("--env", action="append", default=[], metavar="KEY=VALUE")
    parser.add_argument("cmd", nargs=argparse.REMAINDER)
    a = parser.parse_args()
    cmd = a.cmd
    if cmd and cmd[0] == "--":
        cmd = cmd[1:]
    if not cmd:
        parser.error("server command required after --")
    extra = dict(kv.split("=", 1) for kv in a.env)
    asyncio.run(run(cmd, a.tool, json.loads(a.args), extra))


if __name__ == "__main__":
    sys.exit(main())
