#!/bin/bash
# Final measurement: the four setups at the same time (one worker each), so
# they share the model API's conditions: AgentGo 0.1.5 + GoMini v5, 0.1.5
# alone, 0.1.6 + GoMini v6, 0.1.6 alone, on the 38 evaluation tasks.
# Usage: phase5.sh <0.1.6 build folder>
cd /c/Users/m7md0/bench
B=${1:?build folder}
P=$PWD/.venv/Scripts/python.exe
log() { echo "[$(date +%H:%M:%S)] $*"; }
stop_all() {
  powershell -NoProfile -Command "Get-CimInstance Win32_Process -Filter \"Name='python.exe'\" | Where-Object { \$_.CommandLine -like '*mcpd.py serve*' } | ForEach-Object { taskkill /PID \$_.ProcessId /T /F | Out-Null }" > /dev/null 2>&1
  sleep 5
}
stop_all
log "bridges"
(AGENTGO_DEV_EXE="C:/Users/m7md0/bench/baseline-0.1.5/agentgo.exe" AGENTGO_CUSTOM_ROOT="C:/Users/m7md0/bench/workers/bw1" \
 BENCH_GOMINI_ROOT="C:/Users/m7md0/bench/gomini-v5" $P mcpd.py serve gomini-sweep 9171 > workers/bridge-bw1.log 2>&1 &)
(AGENTGO_DEV_EXE="C:/Users/m7md0/bench/baseline-0.1.5/agentgo.exe" AGENTGO_CUSTOM_ROOT="C:/Users/m7md0/bench/workers/bs1" \
 $P mcpd.py serve custom 9181 > workers/bridge-bs1.log 2>&1 &)
(AGENTGO_DEV_EXE="C:/Users/m7md0/bench/$B/agentgo.exe" AGENTGO_CUSTOM_ROOT="C:/Users/m7md0/bench/workers/gw1" \
 BENCH_GOMINI_ROOT="C:/Users/m7md0/bench/gomini-v6" $P mcpd.py serve gomini-sweep 9121 > workers/bridge-gw1.log 2>&1 &)
(AGENTGO_DEV_EXE="C:/Users/m7md0/bench/$B/agentgo.exe" AGENTGO_CUSTOM_ROOT="C:/Users/m7md0/bench/workers/gs1" \
 $P mcpd.py serve custom 9131 > workers/bridge-gs1.log 2>&1 &)
until [ "$(cat workers/bridge-bw1.log workers/bridge-bs1.log workers/bridge-gw1.log workers/bridge-gs1.log 2>/dev/null | grep -c ready)" -ge 4 ]; do sleep 3; done
log "running the four setups together"
$P v016/gbench.py run --label final-gm015-$B --pool bw --suites mw-eval,spa --workers 1 > results/v016-final-gm015.log 2>&1 &
$P v016/gbench.py run --label final-solo015-$B --pool bs --suites mw-eval,spa --workers 1 > results/v016-final-solo015.log 2>&1 &
$P v016/gbench.py run --label final-gm016v6-$B --pool gw --suites mw-eval,spa --workers 1 > results/v016-final-gm016v6.log 2>&1 &
$P v016/gbench.py run --label final-solo016-$B --pool gs --suites mw-eval,spa --workers 1 > results/v016-final-solo016.log 2>&1 &
wait
stop_all
for l in final-gm015-$B final-solo015-$B final-gm016v6-$B final-solo016-$B; do echo "== $l"; $P v016/gbench.py summary --label $l | head -3; done
log "phase 5 done"
