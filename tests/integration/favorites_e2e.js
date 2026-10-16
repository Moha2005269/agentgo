// The favorites bar (star, chips, remove, turn off), pinned extension buttons,
// and a panel sliding away when the person clicks the page.
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { execFileSync } = require("node:child_process");
const { launch, operator, cdpEval, delay, reservePort, serve } = require("./harness");

let clickInfo = "";
const until = async (what, check, tries = 60) => {
  for (let i = 0; i < tries; i += 1) {
    const value = await check();
    if (value) return value;
    await delay(150);
  }
  throw new Error(`timed out waiting for ${what}`);
};

// A left-button press delivered to the biggest page window of the browser: the
// message the system sends when the person clicks the website.
const CLICK_SCRIPT = `
Add-Type -TypeDefinition @"
using System; using System.Text; using System.Runtime.InteropServices;
public static class Cl {
  [StructLayout(LayoutKind.Sequential)] public struct R { public int L, T, Rt, B; }
  public delegate bool P(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out R r);
  [DllImport("user32.dll")] static extern bool EnumWindows(P p, IntPtr l);
  [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr h, P p, IntPtr l);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] static extern int GetClassName(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  public static string Press(uint want) {
    IntPtr best = IntPtr.Zero; long area = 0;
    P child = (h, l) => { var s = new StringBuilder(256); GetClassName(h, s, 256); R r;
      if (s.ToString() == "Chrome_RenderWidgetHostHWND" && GetWindowRect(h, out r)) { long a = (long)(r.Rt - r.L) * (r.B - r.T); if (a > area) { area = a; best = h; } } return true; };
    EnumWindows((h, l) => { uint pid; GetWindowThreadProcessId(h, out pid); if (pid == want) EnumChildWindows(h, child, IntPtr.Zero); return true; }, IntPtr.Zero);
    if (best == IntPtr.Zero) return "no page window";
    PostMessage(best, 0x0201, (IntPtr)1, (IntPtr)((60 << 16) | 60));
    PostMessage(best, 0x0202, IntPtr.Zero, (IntPtr)((60 << 16) | 60));
    uint bp; uint tid = GetWindowThreadProcessId(best, out bp); R rr; GetWindowRect(best, out rr);
    bool ok = PostMessage(best, 0x0200, IntPtr.Zero, (IntPtr)((61 << 16) | 61));
    return "posted to " + best + " tid=" + tid + " pid=" + bp + " ok=" + ok + " rect=" + rr.L + "," + rr.T + "," + rr.Rt + "," + rr.B;
  }
}
"@
Write-Output ([Cl]::Press(PID))
`;
const clickPage = (pid) => {
  const out = execFileSync("powershell", ["-NoProfile", "-NonInteractive", "-Command", CLICK_SCRIPT.replace("PID", String(pid))], { stdio: ["ignore", "pipe", "ignore"], encoding: "utf8" });
  clickInfo = out.trim();
};

(async () => {
  const port = await reservePort();
  const site = await serve();
  const browser = launch({ args: ["--url", site.url("fixture.html"), "--mode", "full"], debugPort: port });
  let failed = false;
  try {
    await browser.call("api.health");
    const settings = await browser.call("settings.get");
    assert.notEqual(settings.favoritesBar, false, "the favorites bar is on by default");
    const chrome = (expression) => cdpEval(port, "chrome.html", expression);

    // 1. The bar is there and the window gives it room.
    assert.equal(await chrome("document.documentElement.clientHeight"), 140);
    await until("the hint", async () => /Star a site/.test(await chrome("document.querySelector('#favs').textContent")));

    // 2. The star adds the page; its chip appears and the star is lit.
    await until("an http tab", async () => (await chrome("activeTab()?.url || ''")).startsWith("http"));
    await chrome("document.querySelector('#star').click()");
    await until("the chip", async () => chrome("document.querySelectorAll('#favs .fav').length === 1 && !document.querySelector('#favs .fav.landing')"), 80);
    assert.equal(await chrome("document.querySelector('#star').classList.contains('on')"), true);
    const marks = await browser.call("bookmark.list");
    assert.equal(marks.items.length, 1);

    // 3. A chip opens its page in the current tab.
    await chrome("document.querySelector('#favs .fav').click()");
    await delay(600);

    // 4. Removing: the chip goes and the star dims.
    await chrome("document.querySelector('#favs .fav-x').click()");
    await until("the chip to go", async () => chrome("document.querySelectorAll('#favs .fav').length === 0"), 80);
    assert.equal((await browser.call("bookmark.list")).items.length, 0);

    // 5. Many favorites: what does not fit goes behind the » button.
    for (let i = 0; i < 40; i += 1) await browser.call("bookmark.add", { url: `https://site${i}.example/`, title: `Site number ${i}` });
    await until("overflow", async () => chrome("(refreshBookmarks(), !document.querySelector('#favs .fav-more')?.hidden)"), 80);

    // 6. Turning the bar off gives the page its room back.
    await operator(port, "settings.set", { settings: { favoritesBar: false } });
    await until("a shorter chrome", async () => (await chrome("document.documentElement.clientHeight")) === 104);
    await operator(port, "settings.set", { settings: { favoritesBar: true } });
    await until("a taller chrome", async () => (await chrome("document.documentElement.clientHeight")) === 140);

    // 7. Extensions: installed ones are pinned to the toolbar and can be hidden or turned off.
    const folder = fs.mkdtempSync(path.join(os.tmpdir(), "agentgo-ext-"));
    fs.writeFileSync(path.join(folder, "manifest.json"), JSON.stringify({
      manifest_version: 3, name: "Quiet Pen", version: "1.2", description: "Draws a thin line.",
      content_scripts: [{ matches: ["<all_urls>"], js: ["a.js"] }],
    }));
    fs.writeFileSync(path.join(folder, "a.js"), "document.title += '!'");
    const installed = await operator(port, "extension.install", { path: folder });
    assert.equal(installed.pinned, true);
    assert.equal(installed.description, "Draws a thin line.");
    await until("the pinned button", async () => chrome("(refreshExtensions(), document.querySelectorAll('#extBar .ib.ext').length === 1)"));
    await operator(port, "extension.disable", { id: installed.id, pinned: true });
    await until("a dimmed button", async () => chrome("(refreshExtensions(), !!document.querySelector('#extBar .ib.ext.off'))"));
    await operator(port, "extension.enable", { id: installed.id, pinned: false });
    await until("no button", async () => chrome("(refreshExtensions(), document.querySelectorAll('#extBar .ib.ext').length === 0)"));
    await operator(port, "ui.open_tool", { tool: "extensions" });
    await until("the panel", async () => cdpEval(port, "tools.html", "document.querySelectorAll('.ext').length === 1"), 80);
    await cdpEval(port, "tools.html", "document.querySelector('[data-ext-pin]').click()");
    await until("pinned again", async () => (await operator(port, "extension.list")).extensions[0].pinned === true);
    await operator(port, "ui.open_tool", { tool: "extensions" });
    await delay(500);

    // 8. A click on the page slides the open panel away.
    await operator(port, "ui.open_tool", { tool: "settings" });
    await until("settings open", async () => (await chrome("openTool")) === "settings");
    await delay(1200);
    clickPage(browser.child.pid);
    await until("the panel to close", async () => (await chrome("openTool")) === null, 40);

    console.log(JSON.stringify({ ok: true }));
  } catch (error) {
    console.error(`${clickInfo} | ${error.message}`);
    failed = true;
  } finally {
    site.close();
    await browser.close();
    process.exitCode = failed ? 1 : 0;
  }
})();
