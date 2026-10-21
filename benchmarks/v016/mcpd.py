"""Keeps one browser MCP server alive and lets the agent call its tools.

  python mcpd.py serve <config> <port>      start the server (background)
  python mcpd.py call <port> <tool> [json]  call a tool, print what it returns
  python mcpd.py tools <port>               list tools (name + schema size)

The output of `call` is the tool result as an MCP host would hand it to the
model (text parts joined; images only noted), so the agent reads exactly what
it would read through a native MCP connection.
"""

import asyncio
import json
import sys
import urllib.request
from pathlib import Path

from aiohttp import web
from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client

ROOT = Path(__file__).parent
AGENTGO = r"C:\Users\m7md0\AppData\Local\Programs\AgentGo\agentgo.exe"
GOMINI_ROOT = r"C:\Users\m7md0\AppData\Local\AgentGo\gomini"
import os as _os
DEV_GOMINI = str(ROOT / "gomini-v5")
DEV = _os.environ.get("AGENTGO_DEV_EXE", r"C:\Users\m7md0\agentgo\dist\AgentGo-0.1.5-win64\agentgo.exe")
CONFIGS = {
    "browser-use": (str(ROOT / ".venv" / "Scripts" / "browser-use.exe"), ["--mcp"],
                    {"ANONYMIZED_TELEMETRY": "false", "BROWSER_USE_LOGGING_LEVEL": "error"}),
    "playwright": ("npx.cmd", ["-y", "@playwright/mcp@0.0.83", "--isolated"], {}),
    "agentgo": (AGENTGO, ["mcp"], {"AGENTGO_DATA_ROOT": str(ROOT / "agentgo-solo"),
                                   "AGENTGO_GOMINI_ROOT": GOMINI_ROOT}),
    "agentgo-gomini": (AGENTGO, ["mcp"], {"AGENTGO_DATA_ROOT": str(ROOT / "agentgo-gomini"),
                                          "AGENTGO_GOMINI_ROOT": GOMINI_ROOT}),
    # The build under test (portable folder from scripts\package.ps1).
    "dev": (DEV, ["mcp"], {"AGENTGO_DATA_ROOT": str(ROOT / "agentgo-dev-solo"),
                           "AGENTGO_TRACE_LISTENERS": str(ROOT / "results" / "listeners.log"),
                           "AGENTGO_GOMINI_ROOT": DEV_GOMINI}),
    "dev-teacher": (DEV, ["mcp"], {"AGENTGO_DATA_ROOT": str(ROOT / "agentgo-dev-teacher"),
                                   "AGENTGO_GOMINI_ROOT": DEV_GOMINI}),
    "dev-teacher2": (DEV, ["mcp"], {"AGENTGO_DATA_ROOT": str(ROOT / "agentgo-dev-teacher2"),
                                   "AGENTGO_GOMINI_ROOT": DEV_GOMINI}),
    "dev-teacher3": (DEV, ["mcp"], {"AGENTGO_DATA_ROOT": str(ROOT / "agentgo-dev-teacher3"),
                                   "AGENTGO_GOMINI_ROOT": DEV_GOMINI}),
    "dev-teacher4": (DEV, ["mcp"], {"AGENTGO_DATA_ROOT": str(ROOT / "agentgo-dev-teacher4"),
                                   "AGENTGO_GOMINI_ROOT": DEV_GOMINI}),
    "dev-teacher5": (DEV, ["mcp"], {"AGENTGO_DATA_ROOT": str(ROOT / "agentgo-dev-teacher5"),
                                   "AGENTGO_GOMINI_ROOT": DEV_GOMINI}),
    # Any profile folder: AGENTGO_CUSTOM_ROOT=<folder>.
    "custom": (DEV, ["mcp"], {"AGENTGO_DATA_ROOT": _os.environ.get("AGENTGO_CUSTOM_ROOT", ""),
                              "AGENTGO_GOMINI_ROOT": DEV_GOMINI}),
    # Any profile and any GoMini folder (a v6 test model):
    # AGENTGO_CUSTOM_ROOT=<profile folder> BENCH_GOMINI_ROOT=<gomini folder>.
    "gomini-sweep": (DEV, ["mcp"], {"AGENTGO_DATA_ROOT": _os.environ.get("AGENTGO_CUSTOM_ROOT", ""),
                                    "AGENTGO_GOMINI_ROOT": _os.environ.get("BENCH_GOMINI_ROOT", DEV_GOMINI)}),
    "dev-gomini": (DEV, ["mcp"], {"AGENTGO_DATA_ROOT": str(ROOT / "agentgo-dev-gomini"),
                                  "AGENTGO_GOMINI_ROOT": DEV_GOMINI}),
}
# Tools left out so every setup is driven by the same model and text only.
BLOCKED = {"retry_with_browser_use_agent", "browser_evaluate", "browser_run_code"}


def render(result) -> str:
    parts = []
    for c in result.content:
        if c.type == "text":
            parts.append(c.text)
        elif c.type == "image":
            parts.append(f"[image {getattr(c, "mime_type", None) or getattr(c, "mimeType", "")}, {len(c.data)} b64 chars]")
        else:
            parts.append(f"[{c.type}]")
    text = "\n".join(parts)
    err = getattr(result, "is_error", None) or getattr(result, "isError", False)
    return ("ERROR: " + text) if err else text


async def serve(config: str, port: int):
    import os
    cmd, args, env = CONFIGS[config]
    params = StdioServerParameters(command=cmd, args=args, env={**os.environ, **env})
    async with stdio_client(params) as (r, w), ClientSession(r, w) as s:
        init = await s.initialize()
        tools = (await s.list_tools()).tools
        lock = asyncio.Lock()

        # Recording (AgentGo only, AGENTGO_RECORD=<file>, turned on per run by
        # POST /context {run, goal, record: true}): before each click, fill,
        # check or select the agent makes, the GoMini candidates for the page
        # (decision.record) and its text are saved with the agent's action, so
        # solved runs become GoMini training records labelled by the agent.
        context = {"run": None, "goal": None, "record": False}

        async def context_h(req):
            context.update(await req.json())
            return web.json_response({"ok": True})

        async def capture(args):
            action = args.get("action")
            values = {}
            if action == "fill" and args.get("text") is not None:
                values = {"text1": str(args["text"])}
            elif action == "select" and args.get("value") is not None:
                values = {"choice1": str(args["value"])}
            params = {"subgoal": context["goal"] or "do the task", "values": values,
                      "completion": "the task is done", "constraints": [], "max": 200}
            try:
                rec = render(await s.call_tool("browser_call", {"method": "decision.record", "params": params}))
                text = render(await s.call_tool("browser_call", {"method": "observe.text", "params": {}}))
                return {"record": json.loads(rec[rec.find("{"):]), "text": text, "params": params}
            except Exception as e:  # recording never breaks the run
                return {"error": str(e)[:300]}

        async def call(req):
            body = await req.json()
            if body["tool"] in BLOCKED:
                return web.json_response({"text": "ERROR: tool not allowed in this benchmark"})
            async with lock:
                args = body.get("args") or {}
                snap = None
                if (context["record"] and os.environ.get("AGENTGO_RECORD") and body["tool"] == "browser_act"
                        and args.get("ref") is not None and args.get("action") in ("click", "fill", "check", "uncheck", "select")):
                    snap = await capture(args)
                try:
                    res = await s.call_tool(body["tool"], args)
                    if snap is not None:
                        line = {"run": context["run"], "goal": context["goal"], "action": args, "reply": render(res)[:800], **snap}
                        with open(os.environ["AGENTGO_RECORD"], "a", encoding="utf-8") as f:
                            f.write(json.dumps(line) + "\n")
                    return web.json_response({"text": render(res)})
                except Exception as e:  # report, keep serving
                    return web.json_response({"text": f"ERROR: {e}"})

        async def tools_h(_):
            return web.json_response({
                "instructions": init.instructions or "",
                "tools": [{"name": t.name, "description": t.description,
                           "inputSchema": getattr(t, "input_schema", None) or getattr(t, "inputSchema", None)} for t in tools if t.name not in BLOCKED]})

        app = web.Application()
        app.add_routes([web.post("/call", call), web.get("/tools", tools_h), web.post("/context", context_h)])
        runner = web.AppRunner(app)
        await runner.setup()
        await web.TCPSite(runner, "127.0.0.1", port).start()
        print(f"{config} MCP ready on {port} ({len(tools)} tools)", flush=True)
        await asyncio.Event().wait()


def post(port, path, obj=None):
    req = urllib.request.Request(f"http://127.0.0.1:{port}{path}",
                                 data=None if obj is None else json.dumps(obj).encode(),
                                 headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=300) as r:
        return json.loads(r.read())


if __name__ == "__main__":
    sys.stdout.reconfigure(encoding="utf-8")
    cmd = sys.argv[1]
    if cmd == "serve":
        asyncio.run(serve(sys.argv[2], int(sys.argv[3])))
    elif cmd == "call":
        args = json.loads(sys.argv[4]) if len(sys.argv) > 4 else {}
        print(post(sys.argv[2], "/call", {"tool": sys.argv[3], "args": args})["text"])
    elif cmd == "tools":
        d = json.loads(urllib.request.urlopen(f"http://127.0.0.1:{sys.argv[2]}/tools").read())
        print(json.dumps(d, indent=1) if "--full" in sys.argv else
              "\n".join(f"{t['name']}: {t['description'][:150]}" for t in d["tools"]))
