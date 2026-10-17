"""Runs techinz/browsers-benchmark against AgentGo (and Playwright's Chrome as
a reference) without the toolkit's other engines. AgentGo is driven through
its own stdio protocol, as an agent would drive it.

    python run_bench.py <benchmark dir> <agentgo.exe> [--engines agentgo,playwright-chrome]
"""
import asyncio
import base64
import json
import os
import subprocess
import sys
import tempfile
import time
import types

BENCH, AGENTGO = sys.argv[1], sys.argv[2]
WANT = (sys.argv[sys.argv.index("--engines") + 1] if "--engines" in sys.argv else "agentgo,playwright-chrome").split(",")
os.chdir(BENCH)
sys.path.insert(0, BENCH)
os.environ.setdefault("PROXY_ENABLED", "false")

import psutil  # noqa: E402
from engines.base import BrowserEngine, NavigationResult  # noqa: E402


class AgentGoEngine(BrowserEngine):
    """AgentGo through `agentgo run --stdio` (JSON lines)."""

    def __init__(self, name="agentgo", proxy=None, **kwargs):
        super().__init__(name, proxy)
        self.proc = None
        self.next_id = 0
        self.root = None

    @property
    def supported_proxy_protocols(self):
        return []

    async def _call(self, method, params=None, timeout=90):
        self.next_id += 1
        my_id = self.next_id
        line = json.dumps({"v": 1, "id": my_id, "method": method, "params": params or {}}) + "\n"
        self.proc.stdin.write(line.encode())
        await self.proc.stdin.drain()
        deadline = time.time() + timeout
        while True:
            remaining = deadline - time.time()
            if remaining <= 0:
                raise TimeoutError(f"{method} timed out")
            raw = await asyncio.wait_for(self.proc.stdout.readline(), remaining)
            if not raw:
                raise RuntimeError("AgentGo exited")
            message = json.loads(raw)
            if message.get("id") != my_id:
                continue
            if not message.get("ok"):
                raise RuntimeError(f"{method}: {message.get('error')}")
            return message.get("result")

    async def start(self):
        self.root = tempfile.mkdtemp(prefix="agentgo-bench-")
        env = dict(os.environ, AGENTGO_DATA_ROOT=self.root, AGENTGO_SKIP_SETUP="1", AGENTGO_NO_PROMPTS="1", AGENTGO_NO_NOTIFY="1")
        self.proc = await asyncio.create_subprocess_exec(
            AGENTGO, "run", "--stdio", "--url", "about:blank", "--mode", "full",
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, env=env, limit=64 * 1024 * 1024)
        await self._call("browser.version", timeout=60)
        await asyncio.sleep(2)
        main = psutil.Process(self.proc.pid)
        self.process_list = [main] + main.children(recursive=True)
        self._start_time = time.time()

    async def stop(self):
        if not self.proc:
            return
        try:
            await self._call("browser.shutdown", timeout=10)
        except Exception:
            pass
        try:
            await asyncio.wait_for(self.proc.wait(), 15)
        except Exception:
            for p in self.process_list:
                try:
                    p.kill()
                except Exception:
                    pass
        self.proc = None

    async def _url(self):
        tabs = (await self._call("tabs.list")).get("tabs", [])
        return next((t.get("url", "") for t in tabs if t.get("active")), "")

    async def navigate(self, url) -> NavigationResult:
        t0 = time.time()
        try:
            await self._call("navigate.url", {"url": url, "observe": "none", "timeout": 60000}, timeout=90)
            await self._call("wait.load", {"timeout": 60000}, timeout=90)
            ok = True
        except Exception:
            ok = False
        # the process list grows with every site (renderers)
        try:
            main = psutil.Process(self.proc.pid)
            self.process_list = [main] + main.children(recursive=True)
        except Exception:
            pass
        return {"url": await self._url(), "load_time": time.time() - t0, "success": ok, "headers": {}}

    async def reload_page(self) -> NavigationResult:
        return await self.navigate(await self._url())

    async def _eval(self, expression):
        out = await self._call("runtime.evaluate", {"expression": expression, "awaitPromise": True, "timeout": 30000}, timeout=45)
        return out.get("value")

    async def locator(self, css_selector):
        html = await self._eval(f"(() => {{ const e = document.querySelector({json.dumps(css_selector)}); return e ? e.outerHTML.slice(0, 20000) : null; }})()")
        return (html is not None, html or "")

    async def get_page_content(self):
        return await self._eval("document.documentElement.outerHTML") or ""

    async def execute_js(self, script):
        return await self._eval(f"(() => {{\n{script}\n}})()")

    async def screenshot(self, path):
        shot = await self._call("observe.screenshot", {"inline": True})
        data = shot.get("data") or shot.get("base64") or shot.get("png")
        if data:
            with open(path, "wb") as f:
                f.write(base64.b64decode(data))
        elif shot.get("path") and os.path.exists(shot["path"]):
            import shutil
            shutil.copy(shot["path"], path)


engines = []
if "agentgo" in WANT:
    engines.append({"class": AgentGoEngine, "params": {"name": "agentgo"}})
if "playwright-chrome" in WANT:
    from engines.playwright.playwright_engine import PlaywrightEngine
    engines.append({"class": PlaywrightEngine, "params": {"headless": False, "name": "playwright-chrome", "browser_type": "chromium"}})

stub = types.ModuleType("config.engines")
stub.engines_config = types.SimpleNamespace(engines=engines)
sys.modules["config.engines"] = stub

import main  # noqa: E402

main.main()
