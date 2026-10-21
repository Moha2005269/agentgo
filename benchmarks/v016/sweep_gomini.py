"""Bug hunt 3: GoMini delegations on MiniWoB++ pages, scored by the page.

For each task and seed, the task's instruction becomes one delegate subgoal
written the way AgentGo's guide asks (name fields as the page does, supply
values, state completion). A final Submit is the planner's step, so the
script presses it after a delegation that says done. The page's reward says
whether the delegation really did its part.

  python sweep_gomini.py --port 9104 --seeds 200-204 --out results/gomini_0.1.4.jsonl
"""

import argparse
import json
import re
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).parent
PY = str(ROOT / ".venv" / "Scripts" / "python.exe")
SERVER = "http://localhost:18765"


def plan(task, goal):
    """(subgoal, values, completion, submit_after) for a task instruction."""
    q = re.findall(r'"([^"]*)"', goal)
    if task == "click-button":
        return f'Click the "{q[0]}" button.', {}, f'"{q[0]}" was clicked', False
    if task == "click-link":
        return f'Click the link "{q[0]}".', {}, f'the link "{q[0]}" was clicked', False
    if task == "click-option":
        m = re.match(r"Select (.+?) and click Submit", goal)
        return f"Select the {m.group(1)} option.", {}, f"{m.group(1)} is selected", True
    if task == "click-checkboxes":
        m = re.match(r"Select (.*?)\s*and click Submit", goal)
        items = m.group(1).strip().rstrip(",")
        if not items or items.lower() == "nothing":
            return "Leave every checkbox unchecked.", {}, "no checkbox is checked", True
        return f"Check the checkboxes {items}; leave the others unchecked.", {}, f"only {items} are checked", True
    if task == "click-tab-2":
        return f'Switch between the tabs until the link "{q[0]}" is shown, then click it.', {}, f'"{q[0]}" was clicked', False
    if task == "enter-text":
        return "Fill the text field with t=t.", {"t": q[0]}, "the text field holds the value", True
    if task == "focus-text":
        return "Click into the textbox.", {}, "the textbox has the focus", False
    if task == "choose-list":
        m = re.match(r"Select (.+?) from the list", goal)
        return "Choose item=item in the list.", {"item": m.group(1)}, "the list shows the item", True
    if task == "enter-date":
        m = re.match(r"Enter (\S+) as the date", goal)
        return "Fill the date field with d=d.", {"d": m.group(1)}, "the date field holds the date", True
    if task == "navigate-tree":
        return f'Expand folders in the file tree until "{q[0]}" is shown, then click "{q[0]}".', {}, f'"{q[0]}" was clicked', False
    if task == "click-collapsible":
        return "Expand the collapsed section.", {}, "the section is expanded", True
    if task == "social-media":
        m = re.match(r'For the user (@\S+), click on the "(.+?)" button', goal)
        return f'In the post by {m.group(1)}, click its "{m.group(2)}" button.', {}, f'"{m.group(2)}" was clicked for {m.group(1)}', False
    if task == "form-sequence-2":
        m = re.match(r'Check the (\w+) radio button and enter the number "(-?\d+)" into the (\w+) textbox', goal)
        return f"Check the {m.group(1)} radio button and fill the {m.group(3)} textbox with n=n.", {"n": m.group(2)}, "both are set", True
    raise KeyError(task)


TASKS = ["click-button", "click-link", "click-option", "click-checkboxes", "click-tab-2", "enter-text",
         "focus-text", "choose-list", "enter-date", "navigate-tree", "click-collapsible", "social-media",
         "form-sequence-2"]


def call(port, tool, args):
    p = subprocess.run([PY, str(ROOT / "mcpd.py"), "call", str(port), tool, json.dumps(args)],
                       capture_output=True, text=True, encoding="utf-8", timeout=180)
    return p.stdout.strip()


def goal_of(task, seed):
    from run_bench import goal_for
    return goal_for(task, seed)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", default="9104")
    ap.add_argument("--seeds", default="200-204")
    ap.add_argument("--tasks", default=",".join(TASKS))
    ap.add_argument("--out", required=True)
    a = ap.parse_args()
    lo, hi = map(int, a.seeds.split("-"))
    out = (ROOT / a.out).open("a", encoding="utf-8")
    for task in a.tasks.split(","):
        for seed in range(lo, hi + 1):
            goal = goal_of(task, seed)
            run = f"gm-{Path(a.out).stem}-{task}-{seed}"
            url = f"{SERVER}/miniwob/{task}.html?seed={seed}&run={run}"
            sub, values, completion, submit = plan(task, goal)
            call(a.port, "browser_open", {"url": url, "view": "none"})
            time.sleep(0.5)
            t0 = time.time()
            reply = call(a.port, "browser_delegate", {"subgoal": sub, "values": values, "completion": completion, "maxSteps": 8})
            ms = round((time.time() - t0) * 1000)
            m = re.search(r'^\{"evidence".*$', reply, re.M) or re.search(r"^\{.*\"status\".*$", reply, re.M)
            d = json.loads(m.group(0)) if m else {"status": "unparsed", "raw": reply[:300]}
            if submit and d.get("status") == "done":
                # The reference the reply names for the planner's step, as a
                # planner would use it; else the page's Submit button.
                named = re.search(r"next step \(\[(\d+)\]", d.get("reason") or "")
                if named:
                    call(a.port, "browser_act", {"action": "click", "ref": int(named.group(1))})
                else:
                    page = call(a.port, "browser_read", {"filter": "Submit"})
                    r = re.search(r'\[(\d+)\] button "Submit"', page)
                    if r:
                        call(a.port, "browser_act", {"action": "click", "ref": int(r.group(1))})
            time.sleep(0.8)
            res = json.loads(urllib_get(f"{SERVER}/result?run={run}"))
            raw = res.get("raw_reward")
            rec = {"task": task, "seed": seed, "goal": goal, "subgoal": sub, "status": d.get("status"),
                   "reason": d.get("reason"), "steps": d.get("steps"), "reward": raw, "ms": ms,
                   "modelMs": d.get("modelMs"), "reply_chars": len(reply)}
            rec["false_done"] = d.get("status") == "done" and not (raw is not None and raw >= 1)
            out.write(json.dumps(rec) + "\n"); out.flush()
            print(f"{task:18} {seed} {str(d.get('status')):11} reward={raw} steps={len(d.get('steps') or [])} {ms}ms {str(d.get('reason'))[:70]}", flush=True)


def urllib_get(url):
    import urllib.request
    with urllib.request.urlopen(url) as r:
        return r.read()


if __name__ == "__main__":
    sys.stdout.reconfigure(encoding="utf-8")
    main()
