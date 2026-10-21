#!/bin/bash
# AgentGo 0.1.6 (with GoMini v6 and without), Playwright MCP and browser-use,
# at the same time, one worker each, on the 38 evaluation tasks.
# Usage: phase6.sh <0.1.6 build folder> <label suffix>
cd /c/Users/m7md0/bench
B=${1:?build}; S=${2:?suffix}
P=$PWD/.venv/Scripts/python.exe
log() { echo "[$(date +%H:%M:%S)] $*"; }
stop_all() {
  powershell -NoProfile -Command "Get-CimInstance Win32_Process -Filter \"Name='python.exe'\" | Where-Object { \$_.CommandLine -like '*mcpd.py serve*' } | ForEach-Object { taskkill /PID \$_.ProcessId /T /F | Out-Null }" > /dev/null 2>&1
  sleep 5
}
stop_all
rm -f workers/bridge-gw1.log workers/bridge-gs1.log workers/bridge-pw.log workers/bridge-bu.log
log "bridges"
(AGENTGO_DEV_EXE="C:/Users/m7md0/bench/$B/agentgo.exe" AGENTGO_CUSTOM_ROOT="C:/Users/m7md0/bench/workers/gw1" \
 BENCH_GOMINI_ROOT="C:/Users/m7md0/bench/gomini-v6" $P mcpd.py serve gomini-sweep 9121 > workers/bridge-gw1.log 2>&1 &)
(AGENTGO_DEV_EXE="C:/Users/m7md0/bench/$B/agentgo.exe" AGENTGO_CUSTOM_ROOT="C:/Users/m7md0/bench/workers/gs1" \
 $P mcpd.py serve custom 9131 > workers/bridge-gs1.log 2>&1 &)
($P mcpd.py serve playwright 9102 > workers/bridge-pw.log 2>&1 &)
($P mcpd.py serve browser-use 9101 > workers/bridge-bu.log 2>&1 &)
until [ "$(cat workers/bridge-gw1.log workers/bridge-gs1.log workers/bridge-pw.log workers/bridge-bu.log 2>/dev/null | grep -c ready)" -ge 4 ]; do sleep 3; done
log "running the four browsers together"
$P v016/gbench.py run --label cmp-gm016v6-$S --pool gw --suites mw-eval,spa --workers 1 > results/v016-cmp-gm016v6.log 2>&1 &
$P v016/gbench.py run --label cmp-solo016-$S --pool gs --suites mw-eval,spa --workers 1 > results/v016-cmp-solo016.log 2>&1 &
$P v016/gbench.py run --label cmp-playwright-$S --pool pw --suites mw-eval,spa --workers 1 > results/v016-cmp-playwright.log 2>&1 &
$P v016/gbench.py run --label cmp-browseruse-$S --pool bu --suites mw-eval,spa --workers 1 > results/v016-cmp-browseruse.log 2>&1 &
wait
stop_all
for l in cmp-gm016v6-$S cmp-solo016-$S cmp-playwright-$S cmp-browseruse-$S; do echo "== $l"; $P v016/gbench.py summary --label $l | head -3; done
log "phase 6 done"
