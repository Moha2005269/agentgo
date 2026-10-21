"""Grading server for the SPA benchmark (heavy client-side rendering).

Serves the task pages in pages/, each a small single-page app that grades
itself: when the episode ends, the page posts its reward here, the same way
grade_server.py does for MiniWoB++. Pages also post diagnostic events
(clicks before hydration, clicks that landed on a row that had re-rendered
under the agent, optimistic rollbacks...) so failures can be explained.

  GET  /spa/<task>[/<client route>]?seed=N&run=ID   the app (deep links too)
  POST /api/echo?ms=M&status=S                      fake backend: the body back after M ms
  POST /report                                      page -> server, first result wins
  POST /event                                       page -> server, diagnostics
  GET  /result?run=ID                               {result, events}
"""

import json
import sys
import threading
import time
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, urlparse

ROOT = Path(__file__).parent
PAGES = ROOT / "pages"
TASKS = {t["task"]: t for t in json.loads((ROOT / "tasks.json").read_text(encoding="utf-8"))}
RESULTS: dict[str, dict] = {}
EVENTS: dict[str, list] = {}
LOCK = threading.Lock()


class Handler(SimpleHTTPRequestHandler):
    def __init__(self, *a, **kw):
        super().__init__(*a, directory=str(PAGES), **kw)

    def log_message(self, *a):
        pass

    def end_headers(self):
        self.send_header("Cache-Control", "no-store")
        super().end_headers()

    def do_GET(self):
        url = urlparse(self.path)
        if url.path == "/result":
            run = parse_qs(url.query).get("run", [""])[0]
            with LOCK:
                return self._json({"result": RESULTS.get(run), "events": EVENTS.get(run, [])})
        parts = url.path.strip("/").split("/")
        if len(parts) >= 2 and parts[0] == "spa":
            task = TASKS.get(parts[1])
            f = PAGES / f"{parts[1]}.html"
            if not task or not f.is_file():
                return self.send_error(404)
            cfg = {"task": parts[1], "target": task.get("target", {}), "base": f"/spa/{parts[1]}"}
            head = (f"<script>window.BENCH_CFG={json.dumps(cfg)};</script>"
                    "<script src=\"/bench.js\"></script>")
            html = f.read_text(encoding="utf-8").replace("<head>", "<head>" + head, 1)
            return self._send(html.encode("utf-8"), "text/html; charset=utf-8")
        return super().do_GET()

    def do_POST(self):
        url = urlparse(self.path)
        n = int(self.headers.get("Content-Length", 0))
        raw = self.rfile.read(n) or b"{}"
        if url.path == "/api/echo":
            q = parse_qs(url.query)
            time.sleep(min(int(q.get("ms", ["0"])[0]), 10000) / 1000)
            status = int(q.get("status", ["200"])[0])
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(raw)))
            self.end_headers()
            self.wfile.write(raw)
            return
        try:
            body = json.loads(raw)
        except ValueError:
            return self.send_error(400)
        body["server_t"] = time.time()
        with LOCK:
            if url.path == "/report":
                RESULTS.setdefault(body.get("run", ""), body)  # first result wins
            elif url.path == "/event":
                EVENTS.setdefault(body.get("run", ""), []).append(body)
            else:
                return self.send_error(404)
        self._json({"ok": True})

    def _send(self, data, ctype):
        self.send_response(200)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def _json(self, obj):
        self._send(json.dumps(obj).encode(), "application/json")


if __name__ == "__main__":
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 18766
    print(f"SPA grading server on http://localhost:{port}", flush=True)
    ThreadingHTTPServer(("127.0.0.1", port), Handler).serve_forever()
