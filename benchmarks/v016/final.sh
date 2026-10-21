#!/bin/bash
# The final build, end to end: smoke tests, the wait fixes on build-poll, the
# GoMini sweep (v5, v6) and the four setups measured together.
cd /c/Users/m7md0/bench
B=${1:?build}
P=$PWD/.venv/Scripts/python.exe
log() { echo "[$(date +%H:%M:%S)] $*"; }
stop_all() {
  powershell -NoProfile -Command "Get-CimInstance Win32_Process -Filter \"Name='python.exe'\" | Where-Object { \$_.CommandLine -like '*mcpd.py serve*' } | ForEach-Object { taskkill /PID \$_.ProcessId /T /F | Out-Null }" > /dev/null 2>&1
  sleep 5
}
log "smoke ($B)"
bash v016/start_smoke.sh $B > /dev/null
PYTHONIOENCODING=utf-8 $P v016/smoke.py 9150 2>&1 | grep -E '"page"|button-delay|scroll-text-2|before_hydration|after_hydration|Error|Trace'
PYTHONIOENCODING=utf-8 $P - <<'EOF'
import json, time, urllib.request, re
def call(tool, args):
    req = urllib.request.Request("http://127.0.0.1:9150/call", data=json.dumps({"tool": tool, "args": args}).encode(), headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=120) as r: return json.loads(r.read())["text"]
run = f"waitfix-{int(time.time())}"
page = call("browser_open", {"url": f"http://localhost:18780/spa/build-poll?seed=7&run={run}"})
t0 = time.time(); w1 = call("browser_wait", {"text": "#418 main Passed", "timeout": 30000}); s1 = round(time.time() - t0, 1)
ref = int(re.search(r'\[(\d+)\] button "Deploy #418"', call("browser_read", {})).group(1))
call("browser_act", {"action": "click", "ref": ref})
t0 = time.time(); w2 = call("browser_wait", {"gone": "Last checked", "timeout": 3000}); s2 = round(time.time() - t0, 1)
print("wait text (row wording):", s1, "s", w1[:80].replace("\n", " "))
print("wait gone with a text:", s2, "s", w2[:80].replace("\n", " "))
EOF
stop_all
log "GoMini sweep"
bash v016/sweep.sh $B final 2>&1 | grep -E "^v[56] "
log "four setups together"
bash v016/phase5.sh $B
log "final done"
