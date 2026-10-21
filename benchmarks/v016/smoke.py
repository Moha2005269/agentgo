"""Smoke test of a build's new behaviour through its own MCP tools (no model):
  python v016/smoke.py <bridge port>
"""
import json
import re
import sys
import time
import urllib.request

PORT = int(sys.argv[1])
SPA = "http://localhost:18780"


def call(tool, args):
    req = urllib.request.Request(f"http://127.0.0.1:{PORT}/call", data=json.dumps({"tool": tool, "args": args}).encode(),
                                 headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=120) as r:
        return json.loads(r.read())["text"]


def result(run):
    with urllib.request.urlopen(f"{SPA}/result?run={run}") as r:
        return (json.loads(r.read()).get("result") or {}).get("reason")


def ref_of(text, *words):
    for line in text.splitlines():
        if all(w in line for w in words):
            m = re.search(r"\[(\d+)\]", line)
            if m:
                return int(m.group(1)), line
    return None, None


out = {}
# S1/S2: key={index} inbox: read, let one item be inserted, click the read ref.
run = f"smoke-inbox-{int(time.time())}"
page = call("browser_open", {"url": f"{SPA}/spa/unkeyed-inbox?seed=7&run={run}"})
ref, line = ref_of(page, "Dismiss", "4471 is overdue")
time.sleep(2.5)
reply = call("browser_act", {"action": "click", "ref": ref})
time.sleep(1)
out["inbox"] = {"line": line, "reply": reply[:300], "page": result(run)}
# S4: virtual list, one call.
run = f"smoke-vlist-{int(time.time())}"
page = call("browser_open", {"url": f"{SPA}/spa/virtual-list?seed=7&run={run}"})
anyref, _ = ref_of(page, "Select")
t0 = time.time()
reply = call("browser_act", {"action": "scroll", "ref": anyref, "until": "SKU-01561"})
ms = round((time.time() - t0) * 1000)
ref, line = ref_of(reply + "\n" + call("browser_read", {"filter": "01561"}), "Select", "01561")
reply2 = call("browser_act", {"action": "click", "ref": ref}) if ref else "no ref"
time.sleep(1)
out["vlist"] = {"scroll_ms": ms, "scroll_reply": reply[:300], "line": line, "page": result(run)}
# S5: wait until Deploy #418 is enabled.
run = f"smoke-build-{int(time.time())}"
page = call("browser_open", {"url": f"{SPA}/spa/build-poll?seed=7&run={run}"})
ref, line = ref_of(page, "Deploy #418")
t0 = time.time()
waited = call("browser_wait", {"enabled": ref, "timeout": 30000})
ms = round((time.time() - t0) * 1000)
reply = call("browser_act", {"action": "click", "ref": ref})
time.sleep(1)
out["build"] = {"line": line, "wait_ms": ms, "wait_reply": waited[:200], "page": result(run)}
# B2: the guide.
out["guide_chars"] = len(call("browser_guide", {}))
print(json.dumps(out, indent=1, ensure_ascii=False))

# MiniWoB++ checks, graded by the page (grade_server on 18765).
MW = "http://localhost:18765"


def mw_result(run):
    with urllib.request.urlopen(f"{MW}/result?run={run}") as r:
        return json.loads(r.read()).get("raw_reward")


more = {}
# Timed pause inside a batch.
run = f"smoke-bd-{int(time.time())}"
page = call("browser_open", {"url": f"{MW}/miniwob/button-delay.html?seed=61&run={run}"})
one, _ = ref_of(page, '"ONE"'); two, _ = ref_of(page, '"TWO"')
text = call("browser_call", {"method": "runtime.evaluate", "params": {"expression": "window.__BENCH_GOAL"}})
secs = float(re.search(r"wait (\d+(?:\.\d+)?) second", text).group(1))
call("browser_batch", {"steps": [{"action": "click", "ref": one}, {"action": "wait", "ms": int(secs * 1000)}, {"action": "click", "ref": two}]})
time.sleep(1)
more["button-delay"] = mw_result(run)
# Scroll inside a textarea, then submit.
run = f"smoke-st-{int(time.time())}"
page = call("browser_open", {"url": f"{MW}/miniwob/scroll-text-2.html?seed=61&run={run}"})
area, _ = ref_of(page, "textbox")
sub, _ = ref_of(page, "Submit")
call("browser_act", {"action": "scroll", "ref": area, "to": "bottom"})
call("browser_act", {"action": "click", "ref": sub})
time.sleep(1)
more["scroll-text-2"] = mw_result(run)
# Menus: the menu item, not its wrapper.
run = f"smoke-cm-{int(time.time())}"
page = call("browser_open", {"url": f"{MW}/miniwob/click-menu.html?seed=61&run={run}"})
more["click-menu-outline"] = [l for l in page.splitlines() if "[" in l][:6]
print(json.dumps(more, indent=1, ensure_ascii=False))

# expect: a click before hydration changes nothing and must fail loudly; the
# same click after hydration is met.
exp = {}
run = f"smoke-exp-{int(time.time())}"
page = call("browser_open", {"url": f"{SPA}/spa/hydration-cart?seed=7&run={run}"})
mug, _ = ref_of(page, "Add to cart", "Blue Ceramic Mug")
exp["before_hydration"] = call("browser_act", {"action": "click", "ref": mug, "expect": {"changes": True, "timeout": 800}})[:220]
time.sleep(3)
exp["after_hydration"] = call("browser_act", {"action": "click", "ref": mug, "expect": {"shows": "Cart (1)"}})[:220]
print(json.dumps(exp, indent=1, ensure_ascii=False))
