"""SPA agent benchmark through Factory droid: the same method as droid_bench.py
(the published 0.1.5 run): one fresh `droid exec` session per task and setup,
gpt-6-luna at medium reasoning, only that setup's browser tools
(MCP:bench-<setup>, see ../droid-run/.factory/mcp.json), the same prompt.

Local tasks (tasks.json) are graded by the page itself (spa_server.py); live
public pages are read-only and graded on the agent's final answer.

  python spa/spa_bench.py run [--only a,b] [--tasks x,y]
  python spa/spa_bench.py summary

FACTORY_API_KEY must be set. The SPA server must run on SERVER.
"""

import json
import os
import re
import statistics as st
import subprocess
import sys
import time
import urllib.request
from pathlib import Path

HERE = Path(__file__).parent
BENCH = HERE.parent
OUT = HERE / "results"
RUNDIR = BENCH / "droid-run"
SERVER = "http://localhost:18780"
MODEL, EFFORT = "gpt-6-luna", "medium"
SETUPS = ["agentgo", "agentgo-gomini", "playwright", "browser-use"]
TIMEOUT_S = 420
PREFIX = os.environ.get("RUN_PREFIX", "spa1")
SESSIONS = Path.home() / ".factory" / "sessions" / "-C-Users-m7md0-bench-droid-run"
LOCAL = json.loads((HERE / "tasks.json").read_text(encoding="utf-8"))
# Live, read-only pages; `expect` are all strings the final answer must contain.
LIVE = [
    {"task": "live-todomvc-react", "url": "https://todomvc.com/examples/react/dist/",
     "goal": "Add three todos: \"buy milk\", \"call mom\" and \"pay rent\". Mark \"call mom\" as completed, then show only the "
             "Active todos. Reply with the exact text of the counter of items left.", "expect": ["2 items left"]},
    {"task": "live-react-dev-search", "url": "https://react.dev/",
     "goal": "Use the site's search to find the useOptimistic hook and open its reference page. Reply with the names of the "
             "two parameters of useOptimistic.", "expect": ["value", "reducer"]},
    {"task": "live-github-code-nav", "url": "https://github.com/facebook/react",
     "goal": "Without using the search, go through the file tree to packages, then shared, and open ReactVersion.js. "
             "Reply with the version string that file exports.", "expect": ["19.3.0"]},
]
# Tool replies that say an element reference was stale or gone.
STALE = re.compile(r"target\.stale|stale (element|ref)|ref(erence)? \S+ (is )?(not found|no longer)|not found in the current page snapshot"
                   r"|element (is )?(not attached|detached)|Element (with )?index \d+ (does not exist|not found)|no longer (in|attached)", re.I)


def page_result(run):
    try:
        with urllib.request.urlopen(f"{SERVER}/result?run={run}", timeout=10) as r:
            return json.loads(r.read())
    except Exception:
        return {}


def session_log(session_id):
    calls, results = [], []
    path = SESSIONS / f"{session_id}.jsonl"
    if not path.exists():
        return calls, results
    for line in path.open(encoding="utf-8"):
        try:
            rec = json.loads(line)
        except ValueError:
            continue
        for part in (rec.get("message") or {}).get("content") or []:
            if not isinstance(part, dict):
                continue
            if part.get("type") == "tool_use":
                calls.append(part.get("name", ""))
            elif part.get("type") == "tool_result":
                c = part.get("content")
                results.append(c if isinstance(c, str) else json.dumps(c))
    return calls, results


def run_one(setup, t):
    run = f"{PREFIX}-{setup}--{t['task']}"
    live = "url" in t
    url = t["url"] if live else f"{SERVER}/spa/{t['task']}?seed={t['seed']}&run={run}"
    prompt = f"Open {url} and do this task on that page: {t['goal']}"
    cmd = ["droid", "exec", "-m", MODEL, "-r", EFFORT, "--auto", "high",
           "--only-tools", f"MCP:bench-{setup}", "-o", "json", prompt]
    started = time.time()
    try:
        proc = subprocess.run(cmd, cwd=RUNDIR, capture_output=True, text=True, encoding="utf-8", timeout=TIMEOUT_S)
        out, timed_out = proc.stdout, False
    except subprocess.TimeoutExpired as e:
        out = e.stdout.decode("utf-8", "replace") if isinstance(e.stdout, bytes) else (e.stdout or "")
        timed_out = True
    seconds = round(time.time() - started, 1)
    result = {}
    for line in out.splitlines()[::-1]:
        try:
            result = json.loads(line)
            break
        except ValueError:
            continue
    time.sleep(1.5)
    final = result.get("result") or ""
    if live:
        page = {}
        ok = all(x.lower() in final.lower() for x in t["expect"])
        raw = 1 if ok else 0
    else:
        page = page_result(run)
        raw = (page.get("result") or {}).get("raw_reward")
        ok = raw is not None and raw >= 1
    calls, results = session_log(result.get("session_id", "")) if result.get("session_id") else ([], [])
    stale = [r[:300] for r in results if STALE.search(r)]
    rec = {"setup": setup, "task": t["task"], "live": live, "run": run, "goal": t["goal"],
           "raw_reward": raw, "success": ok, "reason": (page.get("result") or {}).get("reason"),
           "events": [(e["kind"], e.get("detail"), e.get("t_ms")) for e in page.get("events", [])],
           "stale_replies": stale, "timed_out": timed_out, "is_error": result.get("is_error"),
           "seconds": seconds, "duration_ms": result.get("duration_ms"), "turns": result.get("num_turns"),
           "browser_calls": sum(1 for n in calls if n.startswith(f"bench-{setup}___")),
           "tools": [n.split("___")[-1] for n in calls], "usage": result.get("usage") or {},
           "session_id": result.get("session_id"), "final": final[:1500], "model": MODEL, "effort": EFFORT}
    with (OUT / "runs.jsonl").open("a", encoding="utf-8") as f:
        f.write(json.dumps(rec) + "\n")
    return rec


def run(args):
    OUT.mkdir(parents=True, exist_ok=True)
    only = args[args.index("--only") + 1].split(",") if "--only" in args else SETUPS
    tasks = LOCAL + LIVE
    if "--tasks" in args:
        wanted = args[args.index("--tasks") + 1].split(",")
        tasks = [t for t in tasks if t["task"] in wanted]
    done = set()
    if (OUT / "runs.jsonl").exists():
        done = {json.loads(l)["run"] for l in (OUT / "runs.jsonl").open(encoding="utf-8")}
    for i, t in enumerate(tasks):
        order = only[i % len(only):] + only[: i % len(only)]  # rotate who goes first
        for setup in order:
            if f"{PREFIX}-{setup}--{t['task']}" in done:
                continue
            r = run_one(setup, t)
            u = r["usage"]
            print(f"{setup:15} {t['task']:24} reward={r['raw_reward']} turns={r['turns']} browser={r['browser_calls']} "
                  f"s={r['seconds']} in={u.get('input_tokens')} cached={u.get('cache_read_input_tokens')} "
                  f"out={u.get('output_tokens')} stale={len(r['stale_replies'])} events={[e[0] for e in r['events']]}"
                  f"{' TIMEOUT' if r['timed_out'] else ''}", flush=True)


def summary():
    rs = [json.loads(l) for l in (OUT / "runs.jsonl").open(encoding="utf-8")]
    out = {}
    u = lambda r, k: (r["usage"] or {}).get(k) or 0
    for setup in SETUPS:
        for name, sel in (("local", lambda r: not r["live"]), ("live", lambda r: r["live"]), ("all", lambda r: True)):
            g = [r for r in rs if r["setup"] == setup and sel(r)]
            if not g:
                continue
            out[f"{setup} {name}"] = {
                "runs": len(g), "solved": sum(r["success"] for r in g),
                "wrong_action": sum(1 for r in g if (r["raw_reward"] or 0) < 0),
                "timeouts": sum(r["timed_out"] for r in g),
                "stale_replies": sum(len(r["stale_replies"]) for r in g),
                "turns_mean": round(st.mean(r["turns"] or 0 for r in g), 1),
                "browser_calls_mean": round(st.mean(r["browser_calls"] for r in g), 1),
                "seconds_median": round(st.median(r["seconds"] for r in g), 1),
                "seconds_mean": round(st.mean(r["seconds"] for r in g), 1),
                "input_total_mean": round(st.mean(u(r, "input_tokens") + u(r, "cache_read_input_tokens") + u(r, "cache_creation_input_tokens") for r in g)),
                "output_mean": round(st.mean(u(r, "output_tokens") for r in g)),
                "credits_mean": round(st.mean(u(r, "factory_credits") for r in g)),
                "thinking_mean": round(st.mean(u(r, "thinking_tokens") for r in g)),
            }
            solved = out[f"{setup} {name}"]["solved"]
            # Efficiency: what one solved task costs (all runs' spend over the solved ones).
            out[f"{setup} {name}"]["input_per_solved"] = round(sum(u(r, "input_tokens") + u(r, "cache_read_input_tokens") + u(r, "cache_creation_input_tokens") for r in g) / solved) if solved else None
            out[f"{setup} {name}"]["credits_per_solved"] = round(sum(u(r, "factory_credits") for r in g) / solved) if solved else None
    (OUT / "summary.json").write_text(json.dumps(out, indent=1))
    for k, v in out.items():
        print(f"{k:22}", v)


if __name__ == "__main__":
    sys.stdout.reconfigure(encoding="utf-8")
    run(sys.argv[2:]) if sys.argv[1] == "run" else summary()
