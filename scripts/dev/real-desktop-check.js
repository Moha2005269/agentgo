// One check on the person's real desktop (needs their permission): an
// agent-started AgentGo opens behind whatever has the focus, never takes it,
// and has a taskbar button named AgentGo. Uses the packaged build when
// AGENTGO_EXE is set.
//   set AGENTGO_TEST_VISIBLE=1 && node scripts/dev/real-desktop-check.js
const path = require("node:path");
const { execFileSync } = require("node:child_process");
const { launch, repo, delay, fixture } = require("../../tests/integration/harness");

const tool = path.join(repo, "target", "debug", "agentgo-tool.exe");
const windows = () => JSON.parse(execFileSync(tool, ["probe", "windows"], { encoding: "utf8" })).windows;
const foreground = () => execFileSync("powershell", ["-NoProfile", "-Command",
  "Add-Type 'using System;using System.Runtime.InteropServices;public class W{[DllImport(\"user32.dll\")]public static extern IntPtr GetForegroundWindow();[DllImport(\"user32.dll\")]public static extern int GetWindowThreadProcessId(IntPtr h,out int p);}';$p=0;[W]::GetWindowThreadProcessId([W]::GetForegroundWindow(),[ref]$p)|Out-Null;$p"], { encoding: "utf8" }).trim();

(async () => {
  process.env.AGENTGO_TEST_VISIBLE = "1";
  const before = foreground();
  const browser = launch({ args: ["--url", fixture("live.html"), "--mode", "full"] });
  const samples = [];
  try {
    await browser.call("browser.health");
    for (let i = 0; i < 10; i += 1) {
      samples.push(foreground());
      await delay(300);
    }
    // The agent works while the window stays behind.
    const view = await browser.call("observe.page");
    await browser.call("act.click", { ref: Number(view.page.match(/\[(\d+)\] button "Press me"/)[1]) });
    samples.push(foreground());
    const ours = windows().find((w) => w.pid === browser.child.pid && w.visible && w.rect.right - w.rect.left > 600);
    const result = {
      foregroundBefore: before,
      browserPid: String(browser.child.pid),
      browserEverForeground: samples.includes(String(browser.child.pid)),
      window: ours && { title: ours.title, visible: ours.visible, minimized: ours.minimized, taskbar: ours.taskbar, active: ours.activeInQueue },
      clickWorked: /pressed/.test((await browser.call("observe.text")).text),
    };
    console.log(JSON.stringify(result, null, 2));
    process.exitCode = !result.browserEverForeground && result.window?.taskbar && result.clickWorked ? 0 : 1;
  } finally {
    await browser.close();
  }
})().catch((error) => { console.error(error); process.exit(1); });
