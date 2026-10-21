"""AgentGo + GoMini benchmark for the 0.1.6 work: the method of droid_bench.py
and spa/spa_bench.py (gpt-6-luna, medium reasoning, one fresh `droid exec`
session per task, only the browser's MCP tools), run by several workers at
once. Worker N drives bridge `bench-gwN` (mcpd.py serve custom 912N), whose
AgentGo writes every GoMini request and answer to its AGENTGO_GOMINI_TRACE
file; each run notes its slice of that file, so a failed run's GoMini
decisions can be found, corrected and used as training data.

Suites:
  mw-eval   the 24 MiniWoB++ tasks of the published 0.1.5 run (seeds 41, 51)
  mw-diag   other MiniWoB++ tasks a text agent can do (--seed N): diagnostics
            and training pool, never used as an evaluation set
  spa       the 14 SPA tasks (spa/tasks.json + 3 live pages)

  python v016/gbench.py run --label base015 --suites mw-eval,spa,mw-diag --seed 61 --workers 4 --trace-dir records/base015
  python v016/gbench.py summary --label base015
"""

import argparse
import json
import queue
import re
import statistics as st
import subprocess
import sys
import threading
import time
import urllib.request
from pathlib import Path

BENCH = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(BENCH / "spa"))
import spa_bench  # noqa: E402  (task list, live checks)

RUNDIR = BENCH / "droid-run"
MW = "http://localhost:18765"
SPA = "http://localhost:18780"
MODEL, EFFORT = "gpt-6-luna", "medium"
# Worker pool: gw = AgentGo + GoMini (bridges 9121-9124), gs = AgentGo alone
# (9131-9134). Set with --pool.
POOL, PORT_BASE = "gw", 9120
TIMEOUT_S = 420
SESSIONS = Path.home() / ".factory" / "sessions" / "-C-Users-m7md0-bench-droid-run"
TASKS_A = ("click-checkboxes-large click-tab-2-hard enter-date login-user-popup search-engine "
           "social-media email-inbox book-flight use-autocomplete navigate-tree form-sequence-2 terminal").split()
TASKS_B = ("login-user email-inbox-forward-nl click-scroll-list copy-paste-2 guess-number simple-arithmetic "
           "click-pie enter-text-dynamic social-media-some enter-password click-checkboxes-transfer simple-algebra").split()
# Text-reachable MiniWoB++ tasks outside both evaluation sets (the 24 above and
# the 13 GoMini sweep types).
DIAG = ("ascending-numbers book-flight-nodelay button-delay buy-ticket choose-date-easy choose-date-medium "
        "choose-date-nodelay choose-date click-button-sequence click-checkboxes-soft click-collapsible-2 click-dialog "
        "click-dialog-2 click-menu click-menu-2 click-tab click-tab-2-easy click-tab-2-medium click-test click-test-2 "
        "click-test-transfer click-widget copy-paste daily-calendar email-inbox-delete email-inbox-forward "
        "email-inbox-forward-nl-turk email-inbox-important email-inbox-nl-turk email-inbox-noscroll email-inbox-reply "
        "email-inbox-star-reply enter-text-2 enter-time find-greatest find-word focus-text-2 form-sequence "
        "form-sequence-3 generate-number hot-cold multi-layouts multi-orderings number-checkboxes odd-or-even "
        "order-food phone-book read-table read-table-2 scroll-text scroll-text-2 sign-agreement social-media-all "
        "stock-market text-transform unicode-test use-autocomplete-nodelay use-spinner").split()
# Replies that report an element reference as stale or gone (errors only).
STALE = re.compile(r"target\.stale|target\.changed|not found in the current page snapshot|Element (with )?index \d+ (does not exist|not found)"
                   r"|reference \d+ does not exist|elements match", re.I)


def http_json(url, timeout=10):
    try:
        with urllib.request.urlopen(url, timeout=timeout) as r:
            return json.loads(r.read())
    except Exception:
        return {}


def bridge_call(port, tool, args):
    req = urllib.request.Request(f"http://127.0.0.1:{port}/call", data=json.dumps({"tool": tool, "args": args}).encode(),
                                 headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=120) as r:
        return json.loads(r.read())["text"]


def set_context(worker, ctx):
    try:
        req = urllib.request.Request(f"http://127.0.0.1:{PORT_BASE + worker}/context", data=json.dumps(ctx).encode(),
                                     headers={"Content-Type": "application/json"})
        urllib.request.urlopen(req, timeout=10).read()
    except Exception:
        pass  # an older bridge without /context: nothing is recorded


def diag_goals(seed, port):
    """BrowserGym instructions for the diagnostic tasks, read once from the page."""
    path = BENCH / "v016" / f"goals_diag_{seed}.json"
    goals = json.loads(path.read_text()) if path.exists() else {}
    for task in DIAG:
        if task in goals:
            continue
        bridge_call(port, "browser_open", {"url": f"{MW}/miniwob/{task}.html?seed={seed}&run=goal-{task}-{seed}"})
        time.sleep(0.5)
        text = bridge_call(port, "browser_call", {"method": "runtime.evaluate", "params": {"expression": "window.__BENCH_GOAL"}})
        try:
            goals[task] = json.loads(text[text.find("{"):])["value"]
        except Exception:
            print("no goal for", task, text[:120], flush=True)
        path.write_text(json.dumps(goals, indent=1))
    return goals


def build_tasks(suites, seed, port):
    tasks = []
    if "mw-eval" in suites:
        goals = json.loads((BENCH / "results" / "goals.json").read_text())
        for names, s in ((TASKS_A, 41), (TASKS_B, 51)):
            tasks += [{"suite": "mw-eval", "task": t, "seed": s, "goal": goals[f"{t}#{s}"]} for t in names]
    if "spa" in suites:
        for t in spa_bench.LOCAL + spa_bench.LIVE:
            tasks.append(dict(t, suite="spa"))
    if "mw-diag" in suites:
        goals = diag_goals(seed, port)
        tasks += [{"suite": "mw-diag", "task": t, "seed": seed, "goal": goals[t]} for t in DIAG if goals.get(t)]
    return tasks


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
                calls.append((part.get("name", ""), part.get("input")))
            elif part.get("type") == "tool_result":
                c = part.get("content")
                results.append(c if isinstance(c, str) else json.dumps(c))
    return calls, results


def lines(path):
    try:
        with open(path, encoding="utf-8") as f:
            return sum(1 for _ in f)
    except FileNotFoundError:
        return 0


def run_one(label, worker, t, trace):
    server = f"bench-{POOL}{worker}"
    run = f"{label}-{t['task']}--{t.get('seed', 0)}"
    live = "url" in t
    if t["suite"] == "spa":
        url = t["url"] if live else f"{SPA}/spa/{t['task']}?seed={t['seed']}&run={run}"
    else:
        url = f"{MW}/miniwob/{t['task']}.html?seed={t['seed']}&run={run}"
    prompt = f"Open {url} and do this task on that page: {t['goal']}"
    cmd = ["droid", "exec", "-m", MODEL, "-r", EFFORT, "--auto", "high",
           "--only-tools", f"MCP:{server}", "-o", "json", prompt]
    trace_from = lines(trace) if trace else 0
    # The bridge records the agent's actions as GoMini training records on
    # diagnostic runs only; evaluation runs keep their timing untouched.
    set_context(worker, {"run": run, "goal": t["goal"], "record": t["suite"] == "mw-diag"})
    started = time.time()
    try:
        proc = subprocess.run(cmd, cwd=RUNDIR, capture_output=True, text=True, encoding="utf-8", timeout=TIMEOUT_S)
        out, timed_out = proc.stdout, False
    except subprocess.TimeoutExpired as e:
        out = e.stdout.decode("utf-8", "replace") if isinstance(e.stdout, bytes) else (e.stdout or "")
        timed_out = True
    seconds = round(time.time() - started, 1)
    set_context(worker, {"run": None, "goal": None, "record": False})
    result = {}
    for line in out.splitlines()[::-1]:
        try:
            result = json.loads(line)
            break
        except ValueError:
            continue
    time.sleep(1.5)
    final = result.get("result") or ""
    events, reason = [], None
    if live:
        ok = all(x.lower() in final.lower() for x in t["expect"])
        raw = 1 if ok else 0
    elif t["suite"] == "spa":
        page = http_json(f"{SPA}/result?run={run}")
        res = page.get("result") or {}
        raw, reason = res.get("raw_reward"), res.get("reason")
        events = [(e["kind"], e.get("detail"), e.get("t_ms")) for e in page.get("events", [])]
        ok = raw is not None and raw >= 1
    else:
        res = http_json(f"{MW}/result?run={run}")
        raw, reason = res.get("raw_reward"), res.get("reason")
        ok = raw is not None and raw >= 1
    calls, results = session_log(result.get("session_id", "")) if result.get("session_id") else ([], [])
    tools = [n.split("___")[-1] for n, _ in calls]
    # Every tool call is followed by its result in the session log.
    delegations = [{"input": inp, "reply": res[:1500]} for (n, inp), res in zip(calls, results) if n.endswith("browser_delegate")]
    u = result.get("usage") or {}
    rec = {"label": label, "pool": POOL, "suite": t["suite"], "task": t["task"], "seed": t.get("seed"), "run": run, "worker": worker,
           "goal": t["goal"], "raw_reward": raw, "success": bool(ok), "reason": reason, "events": events,
           "stale_replies": [r[:300] for r in results if STALE.search(r[:400])],
           "timed_out": timed_out, "is_error": result.get("is_error"), "seconds": seconds,
           "turns": result.get("num_turns"), "browser_calls": sum(1 for n in tools if n.startswith("browser_")),
           "tools": tools, "delegations": delegations, "usage": u, "session_id": result.get("session_id"),
           "final": final[:1500], "trace": str(trace) if trace else None, "trace_lines": [trace_from, lines(trace) if trace else 0],
           "model": MODEL, "effort": EFFORT}
    return rec


def run(a):
    out = BENCH / "results" / "v016" / a.label
    out.mkdir(parents=True, exist_ok=True)
    tasks = build_tasks(a.suites.split(","), a.seed, PORT_BASE + 1)
    if a.tasks:
        tasks = [t for t in tasks if t["task"] in a.tasks.split(",")]
    if a.goal_suffix:  # a side check (form-sequence-2 with its Submit said), never an official run
        tasks = [dict(t, goal=t["goal"].rstrip() + " " + a.goal_suffix) for t in tasks]
    done = set()
    if (out / "runs.jsonl").exists():
        done = {json.loads(l)["run"] for l in (out / "runs.jsonl").open(encoding="utf-8")}
    q = queue.Queue()
    for t in tasks:
        if f"{a.label}-{t['task']}--{t.get('seed', 0)}" not in done:
            q.put(t)
    print(f"{q.qsize()} runs on {a.workers} workers", flush=True)
    lock = threading.Lock()

    def work(n):
        trace = Path(a.trace_dir) / f"gw{n}.jsonl" if a.trace_dir else None
        while True:
            try:
                t = q.get_nowait()
            except queue.Empty:
                return
            r = run_one(a.label, n, t, trace)
            with lock:
                with (out / "runs.jsonl").open("a", encoding="utf-8") as f:
                    f.write(json.dumps(r) + "\n")
                u = r["usage"]
                print(f"gw{n} {r['suite']:8} {r['task']:26} reward={r['raw_reward']} turns={r['turns']} calls={r['browser_calls']} "
                      f"deleg={len(r['delegations'])} s={r['seconds']} in={(u.get('input_tokens') or 0) + (u.get('cache_read_input_tokens') or 0)} "
                      f"stale={len(r['stale_replies'])}{' TIMEOUT' if r['timed_out'] else ''}", flush=True)

    threads = [threading.Thread(target=work, args=(n,)) for n in range(1, a.workers + 1)]
    for th in threads:
        th.start()
    for th in threads:
        th.join()


def stats(g):
    u = lambda r, k: (r["usage"] or {}).get(k) or 0
    inp = lambda r: u(r, "input_tokens") + u(r, "cache_read_input_tokens") + u(r, "cache_creation_input_tokens")
    solved = sum(r["success"] for r in g)
    return {"runs": len(g), "solved": solved, "wrong_action": sum(1 for r in g if (r["raw_reward"] or 0) < 0),
            "timeouts": sum(r["timed_out"] for r in g), "stale": sum(len(r["stale_replies"]) for r in g),
            "turns": round(st.mean(r["turns"] or 0 for r in g), 1), "calls": round(st.mean(r["browser_calls"] for r in g), 1),
            "delegations": round(st.mean(len(r["delegations"]) for r in g), 2),
            "median_s": round(st.median(r["seconds"] for r in g), 1), "mean_s": round(st.mean(r["seconds"] for r in g), 1),
            "input": round(st.mean(map(inp, g))), "output": round(st.mean(u(r, "output_tokens") for r in g)),
            "credits": round(st.mean(u(r, "factory_credits") for r in g)),
            "input_per_solved": round(sum(map(inp, g)) / solved) if solved else None,
            "credits_per_solved": round(sum(u(r, "factory_credits") for r in g) / solved) if solved else None}


def summary(a):
    out = BENCH / "results" / "v016" / a.label
    rs = [json.loads(l) for l in (out / "runs.jsonl").open(encoding="utf-8")]
    res = {}
    for suite in sorted({r["suite"] for r in rs}):
        res[suite] = stats([r for r in rs if r["suite"] == suite])
    res["all"] = stats(rs)
    (out / "summary.json").write_text(json.dumps(res, indent=1))
    for k, v in res.items():
        print(f"{k:9}", v)
    for r in rs:
        if not r["success"]:
            print("FAIL", r["suite"], r["task"], r["raw_reward"], (r["reason"] or "")[:60], "|", r["final"][:120].replace("\n", " "))


if __name__ == "__main__":
    sys.stdout.reconfigure(encoding="utf-8")
    p = argparse.ArgumentParser()
    p.add_argument("cmd")
    p.add_argument("--label", required=True)
    p.add_argument("--suites", default="mw-eval,spa")
    p.add_argument("--seed", type=int, default=61)
    p.add_argument("--tasks")
    p.add_argument("--workers", type=int, default=4)
    p.add_argument("--trace-dir")
    p.add_argument("--goal-suffix")
    p.add_argument("--pool", default="gw", choices=["gw", "gs", "bw", "bs", "pw", "bu", "rw", "rs"])
    a = p.parse_args()
    POOL, PORT_BASE = a.pool, {"gw": 9120, "gs": 9130, "bw": 9170, "bs": 9180, "pw": 9101, "bu": 9100, "rw": 9190, "rs": 9191}[a.pool]
    run(a) if a.cmd == "run" else summary(a)
