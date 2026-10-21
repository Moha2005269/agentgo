#!/bin/bash
# GoMini sweep (13 held-out MiniWoB++ types x 5 seeds) with v5 and v6 on one
# AgentGo build, one model at a time. Usage: sweep.sh <build folder> <label>
cd /c/Users/m7md0/bench
B=${1:?build}; L=${2:?label}
P=$PWD/.venv/Scripts/python.exe
for v in v5 v6; do
  d=workers/sweep-$v; mkdir -p $d/config $d/data; cp agentgo-dev-gomini/config/settings.json $d/config/; cp agentgo-dev-gomini/data/profiles.json $d/data/
  rm -f workers/bridge-sweep-$v.log results/v016/sweep-$L-$v.jsonl
  (AGENTGO_DEV_EXE="C:/Users/m7md0/bench/$B/agentgo.exe" AGENTGO_CUSTOM_ROOT="C:/Users/m7md0/bench/workers/sweep-$v" \
   BENCH_GOMINI_ROOT="C:/Users/m7md0/bench/gomini-$v" AGENTGO_GOMINI_TRACE="C:/Users/m7md0/bench/records/sweep-$L-$v.jsonl" \
   $P mcpd.py serve gomini-sweep 9160 > workers/bridge-sweep-$v.log 2>&1 &)
  until grep -q ready workers/bridge-sweep-$v.log 2>/dev/null; do sleep 3; done
  $P sweep_gomini.py --port 9160 --seeds 200-204 --out results/v016/sweep-$L-$v.jsonl > results/v016/sweep-$L-$v.log 2>&1
  powershell -NoProfile -Command "Get-CimInstance Win32_Process -Filter \"Name='python.exe'\" | Where-Object { \$_.CommandLine -like '*mcpd.py serve*' } | ForEach-Object { taskkill /PID \$_.ProcessId /T /F | Out-Null }" > /dev/null 2>&1
  sleep 5
  python -c "
import json,collections
rs=[json.loads(l) for l in open('results/v016/sweep-$L-$v.jsonl',encoding='utf-8')]
by=collections.defaultdict(lambda:[0,0]); fd=0
for r in rs:
  ok=(r.get('reward') or 0)>=1; by[r['task']][0]+=ok; by[r['task']][1]+=1; fd+=bool(r.get('false_done'))
print('$v', sum(a for a,b in by.values()),'/',sum(b for a,b in by.values()),'false done',fd, {k:f'{a}/{b}' for k,(a,b) in by.items()})"
done
