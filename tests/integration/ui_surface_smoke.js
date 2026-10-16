// The human-facing surfaces, driven for real: New Tab page (navigation and
// live data), browser chrome (titles, bookmark star, agent controls), tools
// panel (history, settings, approvals) and the trust boundary between them.
// Pass a directory as argv[2] to keep screenshots of each surface.
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { launch, cdpEval, operator, serve, reservePort, fixture, rejects, refOf, delay } = require("./harness");

const landing = "agentgo://shell/index.html?embedded=1";
const shotDir = process.argv[2] ? path.resolve(process.argv[2]) : null;

// A surface that is still loading throws while it is read (its elements do
// not exist yet); that counts as "not ready", not as a failure.
async function until(read, done, what, timeoutMs = 15_000) {
  const started = Date.now();
  let value;
  for (;;) {
    try {
      value = await read();
      if (done(value)) return value;
    } catch (error) {
      value = `${error.message}`.split("\n")[0];
    }
    if (Date.now() - started > timeoutMs) throw new Error(`timed out waiting for ${what}: ${JSON.stringify(value)}`);
    await delay(120);
  }
}

(async () => {
  const port = await reservePort();
  const site = await serve();
  const browser = launch({ args: ["--url", landing], debugPort: port });
  const chrome = (js) => cdpEval(port, "chrome.html", js);
  const tools = (js) => cdpEval(port, "tools.html", js);
  const newtab = (js) => cdpEval(port, "index.html", js);
  const shot = async (match, name) => {
    if (!shotDir) return;
    fs.mkdirSync(shotDir, { recursive: true });
    const target = (await (await fetch(`http://127.0.0.1:${port}/json/list`)).json()).find((t) => t.url.includes(match));
    const socket = new WebSocket(target.webSocketDebuggerUrl);
    await new Promise((resolve) => socket.addEventListener("open", resolve, { once: true }));
    const data = await new Promise((resolve) => {
      socket.addEventListener("message", (event) => resolve(JSON.parse(event.data).result?.data));
      socket.send(JSON.stringify({ id: 1, method: "Page.captureScreenshot", params: { format: "png" } }));
    });
    socket.close();
    if (data) fs.writeFileSync(path.join(shotDir, name), Buffer.from(data, "base64"));
  };
  let failed = false;
  try {
    // --- New Tab page: real data, real navigation, read-only bridge --------
    await until(() => newtab("document.querySelector('#footVersion').textContent"), (v) => v === "0.1.0", "live New Tab state");
    assert.match(await newtab("document.querySelector('#agentChipText').textContent"), /agent connected/);
    assert.equal(await newtab("document.querySelectorAll('.scut').length"), 8);
    assert.deepEqual(
      await newtab("[...document.querySelectorAll('.scut')].map(a => a.href).slice(0, 3)"),
      ["https://github.com/", "https://mail.google.com/", "https://www.figma.com/files/"],
    );
    // No simulated chrome, loader, task log or status numbers remain.
    assert.equal(await newtab("document.querySelectorAll('#loadbar,.statusbar,#sidepanel,#view-generic,#log,#btnDelegate').length"), 0);
    // The page may read its summary and nothing else.
    assert.equal(await newtab("fetch('agentgo://shell/__bridge?method=settings.set&params=%7B%22theme%22%3A%22light%22%7D').then(r => r.status)"), 403);
    assert.equal(await newtab("fetch('agentgo://shell/__bridge?method=agent.stop&params=%7B%7D').then(r => r.status)"), 403);
    await shot("index.html", "newtab.png");

    // Typing an address in the page omnibox really navigates the tab.
    await newtab(`document.querySelector('#omniInput').value = ${JSON.stringify(site.url("fixture.html"))}; document.querySelector('#omniForm').requestSubmit(); true`);
    const loaded = await until(() => browser.call("tabs.status"), (s) => s.url === site.url("fixture.html") && !s.loading && s.title, "omnibox navigation");
    assert.equal(loaded.title, "AgentGo fixture — interaction page");

    // A web page cannot reach the bridge at all.
    const fromWeb = await cdpEval(port, "fixture.html", "fetch('agentgo://shell/__bridge?method=newtab.state&params=%7B%7D').then(r => r.status, e => 'blocked')");
    assert(fromWeb === "blocked" || fromWeb === 403, `web content reached the bridge: ${fromWeb}`);

    // --- Chrome: real title, secure state, bookmark star -------------------
    await until(() => chrome("document.querySelector('.tab.active .tab-title').textContent"), (t) => t === "AgentGo fixture — interaction page", "tab title");
    assert.equal(await chrome("document.querySelector('#address').value"), site.url("fixture.html"));
    assert.equal(await chrome("document.querySelector('#securityText').textContent"), "Not secure");
    assert.equal(await chrome("document.querySelector('.tab.active .tab-agent')?.textContent"), "Agent");
    await chrome("document.querySelector('#star').click(); true");
    await until(() => browser.call("bookmark.list"), (list) => list.items.length === 1, "bookmark saved");
    await until(() => chrome("document.querySelector('#star').classList.contains('on')"), Boolean, "star state");

    // Address bar: a plain query resolves to a Google search URL (checked
    // through the same resolver, without leaving for the network).
    assert.match((await operator(port, "search.resolve", { input: "agent browser" })).url, /^https:\/\/www\.google\.com\/search\?q=agent%20browser$/);

    // New tab from the chrome opens the New Tab page, which now lists the
    // bookmark and the visited page from real stores.
    await chrome("document.querySelector('#newTab').click(); true");
    await until(() => chrome("document.querySelectorAll('.tab').length"), (n) => n === 2, "second tab");
    await until(() => newtab("(document.querySelector('#bookmarkRows')?.textContent || '') + '|' + (document.querySelector('#recentRows')?.textContent || '')"),
      (t) => t.split("|").every((part) => part.includes("AgentGo fixture — interaction page")), "bookmarks and recent rows");
    assert.equal(await chrome("document.querySelector('#securityText').textContent"), "AgentGo");
    await shot("chrome.html", "chrome.png");

    // --- Tools: history, settings ------------------------------------------
    await chrome("document.querySelector('#openHistory').click(); true");
    await until(() => tools("document.body.dataset.open === 'true' && document.querySelectorAll('[data-open-url]').length"), (n) => n >= 1, "history panel");
    assert.match(browser.stderr(), /AG-RUNTIME tools-overlay-restacked/);
    await shot("tools.html", "tools-history.png");
    await tools("window.__agentgoRequestClose(); true");
    await delay(350);

    await chrome("document.querySelector('#openSettings').click(); true");
    await until(() => tools("document.body.classList.contains('settings') && document.querySelectorAll('[data-setting]').length"), (n) => n > 5, "settings panel");
    await shot("tools.html", "tools-settings.png");
    await tools("document.querySelector('[data-setting=\"theme\"][data-value=\"light\"]').click(); true");
    await until(() => chrome("document.documentElement.dataset.theme"), (t) => t === "light", "chrome theme");
    await until(() => newtab("document.documentElement.dataset.theme"), (t) => t === "light", "New Tab theme");
    assert.equal((await browser.call("settings.get")).theme, "light");
    await tools("document.querySelector('[data-setting=\"theme\"][data-value=\"dark\"]').click(); true");
    await until(() => chrome("document.documentElement.dataset.theme"), (t) => t === "dark", "chrome theme back");
    await tools("window.__agentgoRequestClose(); true");
    await delay(350);

    // --- Agent control from the chrome and the panel ------------------------
    await until(() => chrome("document.querySelector('#agentText').textContent"), (t) => /agent connected$/.test(t), "agent pill");
    assert.equal(await chrome("document.querySelector('#agentStop').hidden"), false);
    const own = (await browser.call("tabs.create", { url: fixture("agent.html") })).tabId;
    const view = await browser.call("navigate.url", { tabId: own, url: fixture("agent.html") });
    const pending = await rejects(browser.call("act.click", { ref: refOf(view.page, "Buy now") }), "approval.required");
    // The panel opens by itself and shows the exact request.
    await until(() => tools("document.body.classList.contains('agent') && document.querySelectorAll('[data-approval]').length"), (n) => n === 2, "approval prompt");
    assert.match(await chrome("document.querySelector('#agentText').textContent"), /1 approval waiting/);
    assert.match(await tools("document.querySelector('.need').textContent"), /Buy now/);
    await shot("tools.html", "tools-agent.png");
    const decision = browser.call("approval.wait", { approvalId: pending.data.approvalId, timeout: 15_000 });
    await tools("document.querySelector('[data-approve=\"true\"]').click(); true");
    assert.equal((await decision).status, "granted");
    await browser.call("act.click", { ref: refOf(view.page, "Buy now"), approvalId: pending.data.approvalId });
    assert.match((await browser.call("observe.text")).text, /PURCHASED/);

    // Pause and Stop buttons in the chrome act on the real control state.
    await chrome("document.querySelector('#agentPause').click(); true");
    await until(() => browser.call("agent.status"), (s) => s.control === "paused", "pause");
    await chrome("document.querySelector('#agentPause').click(); true");
    await until(() => browser.call("agent.status"), (s) => s.control === "running", "resume");
    await chrome("document.querySelector('#agentStop').click(); true");
    await until(() => browser.call("agent.status"), (s) => s.control === "stopped", "stop");
    await rejects(browser.call("act.click", { x: 5, y: 5 }), "unauthorized");
    assert.equal(await chrome("document.querySelector('#pauseText').textContent"), "Allow agent");
    await chrome("document.querySelector('#agentPause').click(); true");
    await until(() => browser.call("agent.status"), (s) => s.control === "running", "allow again");

    console.log(JSON.stringify({ ok: true, screenshots: shotDir }));
  } catch (error) {
    console.error(error);
    failed = true;
  } finally {
    const code = await browser.close();
    site.close();
    process.exitCode = failed || code !== 0 ? 1 : 0;
  }
})();
