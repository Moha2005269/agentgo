// The everyday browser around the pages: the settings a person expects
// (startup, search engine, privacy, languages, downloads folder), the find
// bar, the zoom indicator and the browser menu. These are the person's own
// controls, so agents may not change them.
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { launch, operator, cdpEval, serve, reservePort, rejects, delay } = require("./harness");

const until = async (what, check, ms = 8000) => {
  for (const end = Date.now() + ms; Date.now() < end; await delay(100)) {
    const value = await check();
    if (value) return value;
  }
  throw new Error(`timed out waiting for ${what}`);
};

(async () => {
  const site = await serve();
  const dataRoot = fs.mkdtempSync(path.join(os.tmpdir(), "agentgo-browser-"));
  const port = await reservePort();
  let browser = launch({ args: ["--url", "about:blank", "--mode", "full", "--allow-evaluate"], debugPort: port, dataRoot });
  const tools = (expression) => cdpEval(port, "tools.html", expression);
  const chrome = (expression) => cdpEval(port, "chrome.html", expression);
  let failed = false;
  try {
    const page = site.url("browser.html");
    await browser.call("navigate.url", { url: page, observe: "none" });
    const tabId = (await browser.call("tabs.status")).tabId;

    // ---- Search engine ------------------------------------------------------
    await rejects(browser.call("settings.set", { settings: { searchEngine: "Bing" } }), "policy.denied");
    await operator(port, "settings.set", { settings: { searchEngine: "DuckDuckGo" } });
    const resolved = await browser.call("search.resolve", { input: "agent go" });
    assert.equal(resolved.url, "https://duckduckgo.com/?q=agent%20go");
    assert.equal(resolved.engine, "DuckDuckGo");
    await until("the address bar to name the engine", () => chrome("document.querySelector('#address').placeholder.includes('DuckDuckGo')"));
    await operator(port, "settings.set", { settings: { searchEngine: "Google" } });
    assert.match((await browser.call("search.resolve", { input: "x y" })).url, /^https:\/\/www\.google\.com\/search\?q=x%20y$/);

    // ---- Zoom indicator -----------------------------------------------------
    await browser.call("page.zoom", { tabId, percent: 125 });
    assert.equal(await until("the zoom chip", () => chrome("(c => !c.hidden && c.textContent)(document.querySelector('#zoomChip'))")), "125%");
    await chrome("document.querySelector('#zoomChip').click()");
    await until("zoom back to 100", async () => (await browser.call("page.zoom", { tabId })).percent === 100);
    await until("the chip to hide", () => chrome("document.querySelector('#zoomChip').hidden"));

    // ---- Find bar (Chromium's own find, with a match count) ----------------
    await operator(port, "ui.open_tool", { tool: "find" });
    await until("the find bar", () => tools("Boolean(document.querySelector('#findText'))"));
    await tools("(i => { i.value = 'apple'; i.dispatchEvent(new Event('input', { bubbles: true })); })(document.querySelector('#findText'))");
    assert.equal(await until("a match count", () => tools("(t => t === '1/3' && t)(document.querySelector('#findCount').textContent)")), "1/3");
    await tools("document.querySelector('[data-find=next]').click()");
    await until("the second match", () => tools("document.querySelector('#findCount').textContent === '2/3'"));
    await tools("(i => { i.value = 'pear'; i.dispatchEvent(new Event('input', { bubbles: true })); })(document.querySelector('#findText'))");
    await until("no match", () => tools("document.querySelector('#findCount').classList.contains('none')"));
    await tools("document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))");
    await until("the find bar to close", () => tools("!document.body.dataset.open"));

    // ---- Menu: zoom from the menu -----------------------------------------
    await operator(port, "ui.open_tool", { tool: "menu" });
    await until("the menu", () => tools("Boolean(document.querySelector('[data-menu=zoom-in]'))"));
    await tools("document.querySelector('[data-menu=zoom-in]').click()");
    await until("zoom 110 from the menu", async () => (await browser.call("page.zoom", { tabId })).percent === 110);
    assert.equal(await until("the menu to show 110 %", () => tools("(z => z && z.textContent === '110%' && z.textContent)(document.querySelector('#menuZoom'))")), "110%");
    await tools("document.querySelector('[data-menu=zoom-reset]').click()");
    await until("zoom reset from the menu", async () => (await browser.call("page.zoom", { tabId })).percent === 100);
    await tools("document.querySelector('[data-menu=bookmarks]').click()");
    await until("the bookmarks panel", () => tools("document.querySelector('#toolTitle').textContent === 'Bookmarks'"));

    // ---- Settings shows every section --------------------------------------
    await operator(port, "ui.open_tool", { tool: "settings" });
    const sections = await until("settings sections", () => tools("(t => t.includes('On startup') && t)([...document.querySelectorAll('.section h3')].map(h => h.textContent).join('|'))"));
    for (const name of ["Appearance", "Agent control", "On startup", "Search engine", "Privacy", "Languages", "Downloads", "System", "About"]) {
      assert(sections.includes(name), `settings has no ${name} section: ${sections}`);
    }

    // ---- Privacy: clear browsing data --------------------------------------
    await browser.call("runtime.evaluate", { expression: "document.cookie = 'kept=1; max-age=3600'; localStorage.setItem('stored', 'yes'); true" });
    assert.equal((await browser.call("storage.cookies_list")).cookies.length, 1);
    await rejects(browser.call("browsing_data.clear", { confirm: true }), "policy.denied");
    await rejects(operator(port, "browsing_data.clear", {}), "approval.required");
    await operator(port, "history.list", {});
    const cleared = await operator(port, "browsing_data.clear", { confirm: true, history: false });
    assert.deepEqual(cleared.cleared, ["cookies", "cache", "siteData"]);
    assert.equal((await browser.call("storage.cookies_list")).cookies.length, 0);
    assert.equal((await browser.call("runtime.evaluate", { expression: "localStorage.getItem('stored')" })).value, null);
    assert((await operator(port, "history.list", { limit: 10 })).items.length > 0, "history was kept");

    // ---- Forms, files, small natives -----------------------------------------
    const submitted = await browser.call("form.submit", { selector: "#order" });
    assert.match(submitted.submitted, /Send it/);
    assert.match((await browser.call("observe.text")).text, /sent first/);
    const outside = await browser.call("file.path_validate", { path: "C:\\Windows\\win.ini" });
    assert.deepEqual([outside.exists, outside.allowed], [true, false]);
    await rejects(browser.call("file.hash", { path: "C:\\Windows\\win.ini" }), "policy.denied");
    assert.equal((await browser.call("permission.list")).default, "refuse");
    assert((await browser.call("browser_shortcut.list")).shortcuts.length > 10);

    // ---- Tabs and window -----------------------------------------------------
    const copy = await browser.call("tab.duplicate", { tabId });
    assert.equal((await browser.call("tabs.status", { tabId: copy.tabId })).url, page);
    await browser.call("tabs.create", { url: "about:blank" });
    assert.equal((await browser.call("tab.mute", { tabId })).muted, true);
    assert.equal((await browser.call("tab.mute", { tabId, muted: false })).muted, false);
    const right = await browser.call("tab.close_to_right", { tabId: copy.tabId });
    assert.equal(right.closed.length, 1);
    const others = await browser.call("tab.close_others", { tabId });
    assert.deepEqual(others.closed, [copy.tabId]);
    assert.equal((await browser.call("tabs.list")).tabs.length, 1);
    const reopened = await browser.call("tab.reopen_closed");
    assert.equal((await browser.call("tabs.status", { tabId: reopened.tabId })).url, page);
    await browser.call("tabs.close", { tabId: reopened.tabId });
    assert.equal((await browser.call("window.fullscreen", { on: true })).fullscreen, true);
    assert.equal((await browser.call("window.fullscreen", { on: false })).fullscreen, false);
    assert.equal((await browser.call("window.restore")).maximized, false);
    await rejects(browser.call("window.always_on_top", { on: true }), "policy.denied");
    assert.equal((await operator(port, "window.always_on_top", { on: false })).alwaysOnTop, false);

    // ---- Startup and languages (read when the browser starts) --------------
    await rejects(browser.call("settings.set", { settings: { languages: "fr" } }), "policy.denied");
    await operator(port, "settings.set", { settings: { languages: "fr-FR,fr", startup: "newtab" } });
    assert.equal((await browser.call("settings.get")).startup, "newtab");
    // Extensions: a Chrome extension's content script and CSS run on the
    // pages it matches, can be switched off, and only the person installs one.
    {
      const extensionDir = require("node:path").join(require("./harness").repo, "tests", "fixtures", "extension");
      await rejects(browser.call("extension.install", { path: extensionDir }), "policy.denied");
      const installed = await operator(port, "extension.install", { path: extensionDir });
      assert.equal(installed.name, "Banner Test");
      assert.deepEqual(installed.permissions, ["storage", "http://127.0.0.1/*"]);
      assert.equal((await browser.call("extension.list")).extensions.length, 1);
      const ran = async () => (await browser.call("runtime.evaluate", { expression: "new Promise((done) => setTimeout(() => { const m = document.getElementById('ext-mark'); done(m ? m.textContent + ' ' + getComputedStyle(m).color : 'none'); }, 400))" })).value;
      await browser.call("navigate.url", { url: site.url("fixture.html") });
      assert.equal(await ran(), "extension ran on /fixture.html rgb(1, 2, 3)");
      await operator(port, "extension.disable", { id: installed.id });
      await browser.call("navigate.url", { url: site.url("fixture.html") });
      assert.equal(await ran(), "none");
      assert.equal((await operator(port, "extension.storage", { id: installed.id, values: { theme: "dark" } })).storage.theme, "dark");
      assert.equal((await operator(port, "extension.uninstall", { id: installed.id })).uninstalled, true);
      await rejects(operator(port, "extension.install", { path: require("node:os").tmpdir() }), "invalid.params");
      // Folder sync: bookmarks travel through the folder, secrets do not.
      const folder = require("node:fs").mkdtempSync(require("node:path").join(require("node:os").tmpdir(), "agentgo-sync-"));
      require("node:fs").writeFileSync(require("node:path").join(folder, "AgentGo-sync.json"), JSON.stringify({ savedAt: 1, bookmarks: [{ url: "https://synced.example/", title: "From the other computer" }] }));
      await rejects(browser.call("sync.start", { folder }), "policy.denied");
      const synced = await operator(port, "sync.start", { folder });
      assert.equal(synced.bookmarksReceived, 1);
      assert((await browser.call("bookmark.list")).items.some((b) => b.url === "https://synced.example/"));
      const written = require("node:fs").readFileSync(require("node:path").join(folder, "AgentGo-sync.json"), "utf8");
      assert(!/sitePermissions|password|token/i.test(written), "secrets went into the sync file");
      assert.equal((await browser.call("sync.status")).syncing, true);
      assert.equal((await operator(port, "sync.stop")).syncing, false);
      // A site in the Start menu needs the person's approval.
      await rejects(browser.call("web_app.install", { url: "https://example.com/", name: "Example" }), "approval.required");
    }

    await browser.close();

    browser = launch({ args: ["--url", page, "--mode", "full"], dataRoot });
    const text = (await browser.call("observe.text", { max: 2000 })).text;
    assert.match(text, /languages=fr-FR,fr/);
    console.log(JSON.stringify({ ok: true }));
  } catch (error) {
    console.error(error);
    failed = true;
  } finally {
    const code = await browser.close();
    site.close();
    fs.rmSync(dataRoot, { recursive: true, force: true });
    process.exitCode = failed || code !== 0 ? 1 : 0;
  }
})();
