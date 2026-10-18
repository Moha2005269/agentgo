// AgentGo never leaves a console window on the desktop. Some launches give the
// browser a console of its own (Chromium's logging, a pinned taskbar button,
// an agent's terminal); the browser hides it. AGENTGO_TEST_CONSOLE (debug
// builds only) makes the browser open one at startup, as those launches do.
const assert = require("node:assert/strict");
const { execFileSync } = require("node:child_process");
const { launch, delay, executable } = require("./harness");

// Visible top-level console windows owned by conhost processes whose parent
// is the given process.
function visibleConsoles(pid) {
  const script = `
Add-Type -Namespace W -Name U -MemberDefinition '[DllImport("user32.dll")] public static extern bool IsWindowVisible(System.IntPtr h);'
$hosts = Get-CimInstance Win32_Process -Filter "ParentProcessId=${pid} AND Name='conhost.exe'" | % { $_.ProcessId }
$n = 0; foreach ($h in $hosts) { $p = Get-Process -Id $h -ErrorAction SilentlyContinue; if ($p -and $p.MainWindowHandle -ne 0 -and [W.U]::IsWindowVisible($p.MainWindowHandle)) { $n++ } }
"$($hosts.Count) $n"`;
  const [hosts, visible] = execFileSync("powershell", ["-NoProfile", "-Command", script], { encoding: "utf8", windowsHide: true }).trim().split(/\s+/).map(Number);
  return { hosts, visible };
}

// Whether an executable is a windows (GUI) program: only those start without
// a console, which is what the product ships (the packaged agentgo.exe).
function isGui(file) {
  const bytes = require("node:fs").readFileSync(file);
  const pe = bytes.readInt32LE(0x3c);
  return bytes.readUInt16LE(pe + 24 + 68) === 2;
}

(async () => {
  if (!isGui(executable)) {
    console.log(`console_e2e skipped: ${executable} is a console program; run it with AGENTGO_EXE=target\\agentgo-sandbox\\debug\\agentgo.exe (build.ps1 -Runtime -Sandbox)`);
    process.exit(0);
  }
  const browser = launch({ args: ["--url", "about:blank"], env: { AGENTGO_TEST_CONSOLE: "1" } });
  let failed = false;
  try {
    await browser.call("tabs.list");
    await delay(2500);
    assert.match(browser.stderr(), /console-hidden/, "the browser did not hide its console");
    const seen = visibleConsoles(browser.child.pid);
    console.log("console_e2e:", JSON.stringify(seen));
    assert.equal(seen.visible, 0, "a console window is still visible");
    console.log("console_e2e: ok");
  } catch (error) {
    failed = true;
    console.error(error);
    console.error(browser.stderr().slice(-2000));
  } finally {
    await browser.close();
    process.exit(failed ? 1 : 0);
  }
})();
