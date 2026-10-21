"""A stdio MCP server that forwards to one running bridge (mcpd.py serve):
python mcp_proxy.py <port>. The agent host sees the browser's own tools,
descriptions and instructions, minus the tools every setup goes without
(screenshots, page script)."""

import asyncio
import json
import sys
import urllib.request

import mcp.types as types
from mcp.server.lowlevel import Server
from mcp.server.stdio import stdio_server

PORT = int(sys.argv[1])
BLOCK = {"browser_take_screenshot", "browser_screenshot", "browser_run_code_unsafe", "browser_run_code",
         "browser_evaluate", "retry_with_browser_use_agent"}


def http(path, obj=None):
    req = urllib.request.Request(f"http://127.0.0.1:{PORT}{path}", data=None if obj is None else json.dumps(obj).encode(),
                                 headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=300) as r:
        return json.loads(r.read())


META = http("/tools")


async def list_tools(ctx, params):
    return types.ListToolsResult(tools=[
        types.Tool(name=t["name"], description=t["description"] or "",
                   input_schema=t["inputSchema"] or {"type": "object", "properties": {}})
        for t in META["tools"] if t["name"] not in BLOCK])


async def call_tool(ctx, params):
    name, arguments = params.name, params.arguments or {}
    if name in BLOCK:
        text = "ERROR: tool not allowed in this benchmark"
    else:
        text = (await asyncio.to_thread(http, "/call", {"tool": name, "args": arguments}))["text"]
    return types.CallToolResult(content=[types.TextContent(type="text", text=text or "(empty)")])


server = Server("browser", version="1.0", instructions=META.get("instructions") or None,
                on_list_tools=list_tools, on_call_tool=call_tool)


async def main():
    async with stdio_server() as (r, w):
        await server.run(r, w, server.create_initialization_options())


asyncio.run(main())
