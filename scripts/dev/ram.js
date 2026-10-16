// Memory of a running AgentGo: starts it (AGENTGO_EXE or the debug build),
// opens a few pages, waits, then sums private bytes of every process it owns.
//   node scripts/dev/ram.js [tabs=3]
"use strict";
const { execFileSync } = require("node:child_process");
const http = require("node:http");
const path = require("node:path");
const { launch, delay, executable } = require("../../tests/integration/harness");

const tabs = Number(process.argv[2] || 3);
const page = (n) => `<title>Page ${n}</title><body>${"<p>lorem ipsum dolor sit amet</p>".repeat(400)}<img width=1 height=1>`;

(async () => {
  const site = http.createServer((q, r) => r.writeHead(200, { "Content-Type": "text/html" }).end(page(q.url))).listen(0, "127.0.0.1");
  await new Promise((resolve) => site.on("listening", resolve));
  const base = `http://127.0.0.1:${site.address().port}`;
  const browser = launch({ args: ["--url", "about:blank", "--ephemeral"] });
  try {
    await browser.call("navigate.url", { url: `${base}/1` });
    for (let i = 2; i <= tabs; i += 1) await browser.call("tabs.create", { url: `${base}/${i}` });
    await delay(6000);
    const script = `$e='${executable.replace(/'/g, "''")}';` +
      `$p=Get-Process | Where-Object { $_.Path -eq $e };` +
      `$by=$p | Group-Object { (Get-CimInstance Win32_Process -Filter "ProcessId=$($_.Id)").CommandLine -replace '.*--type=([a-z-]+).*','$1' } | ForEach-Object { '{0}={1:N0}MB' -f ($_.Name -replace '^.*agentgo.exe.*$','browser'), (($_.Group | Measure-Object PrivateMemorySize64 -Sum).Sum/1MB) };` +
      `'{0} processes, private {1:N0} MB, working set {2:N0} MB' -f $p.Count, (($p | Measure-Object PrivateMemorySize64 -Sum).Sum/1MB), (($p | Measure-Object WorkingSet64 -Sum).Sum/1MB); $by`;
    console.log(execFileSync("powershell", ["-NoProfile", "-Command", script], { encoding: "utf8" }).trim());
  } finally {
    await browser.close();
    site.close();
  }
})().catch((e) => { console.error(e); process.exit(1); });
