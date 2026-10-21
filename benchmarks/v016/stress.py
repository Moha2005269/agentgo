"""Clicks that race a live list (no model): N tries each on the keyed
leaderboard (rows re-sort every 1.2 s) and the key={index} inbox (a row
inserted on top every 2 s). python v016/stress.py <bridge port> [N]"""
import json
import random
import re
import sys
import time
import urllib.request

PORT, N = int(sys.argv[1]), int(sys.argv[2]) if len(sys.argv) > 2 else 20
SPA = "http://localhost:18780"


def call(tool, args):
    req = urllib.request.Request(f"http://127.0.0.1:{PORT}/call", data=json.dumps({"tool": tool, "args": args}).encode(),
                                 headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=120) as r:
        return json.loads(r.read())["text"]


def reason(run):
    with urllib.request.urlopen(f"{SPA}/result?run={run}") as r:
        return (json.loads(r.read()).get("result") or {}).get("reason")


rng = random.Random(7)
for task, pattern, want in [("keyed-leaderboard", r'\[(\d+)\] button "Follow" in "\d+ Marta Kowalska ', "followed Marta Kowalska"),
                            ("unkeyed-inbox", r'\[(\d+)\] button "Dismiss" in "[^"]*4471 is overdue', "dismissed Invoice #4471 is overdue")]:
    ok = 0
    for i in range(N):
        run = f"stress-{task}-{i}-{int(time.time() * 1000)}"
        page = call("browser_open", {"url": f"{SPA}/spa/{task}?seed=7&run={run}"})
        ref = int(re.search(pattern, page).group(1))
        time.sleep(rng.uniform(0.2, 3.0))
        call("browser_act", {"action": "click", "ref": ref})
        time.sleep(0.8)
        ok += reason(run) == want
    print(f"{task}: {ok}/{N}", flush=True)
