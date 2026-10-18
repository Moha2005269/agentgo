// Does starting AgentGo the way a person does open a console window?
// Starts `agentgo-browser.exe` from a build folder with a throwaway data
// root, waits, and reports whether the browser process (`agentgo.exe run`)
// owns a console host. Run it on a private desktop:
//   agentgo-tool quiet-run -- node scripts/dev/console-probe.js <folder>
const { spawn, execFileSync } = require("node:child_process");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const folder = process.argv[2];
const data = fs.mkdtempSync(path.join(os.tmpdir(), "agentgo-test-console-"));
const direct = process.argv[3] === "direct";
spawn(path.join(folder, direct ? "agentgo.exe" : "agentgo-browser.exe"), direct ? ["run"] : ["--url", "about:blank"], {
  env: { ...process.env, AGENTGO_DATA_ROOT: data, AGENTGO_SKIP_SETUP: "1", AGENTGO_NO_PROMPTS: "1", AGENTGO_NO_NOTIFY: "1" },
  detached: true, stdio: "ignore", windowsHide: false,
}).unref();
setTimeout(() => {
  const rows = JSON.parse(execFileSync("powershell", ["-NoProfile", "-Command",
    "Get-CimInstance Win32_Process | Select ProcessId,ParentProcessId,Name,ExecutablePath,CommandLine | ConvertTo-Json -Compress"], { encoding: "utf8", maxBuffer: 64 << 20 }));
  const browser = rows.find((p) => p.Name === "agentgo.exe" && (p.ExecutablePath || "").startsWith(folder) && / run\b/.test(p.CommandLine || ""));
  const consoles = browser ? rows.filter((p) => p.Name === "conhost.exe" && p.ParentProcessId === browser.ProcessId) : [];
  console.log(JSON.stringify({ browser: browser ? browser.ProcessId : null, console: consoles.length > 0 }));
  for (const p of rows) if ((p.ExecutablePath || "").startsWith(folder)) { try { process.kill(p.ProcessId); } catch (_) {} }
  setTimeout(() => { try { fs.rmSync(data, { recursive: true, force: true }); } catch (_) {} process.exit(0); }, 2000);
}, 12000);
