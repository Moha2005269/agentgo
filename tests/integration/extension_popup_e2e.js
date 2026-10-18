// An extension's own UI: clicking its toolbar button opens its pop-up (and
// the panel's Options its options page) in a small window, served from the
// extension's own origin agentgo-ext://<id>/ with chrome.storage (shared with
// its content scripts), chrome.i18n and chrome.tabs. The pop-up cannot use
// AgentGo's trusted bridge, and a website cannot load an extension's files.
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const http = require("node:http");
const { launch, cdpEval, delay, reservePort, rejects } = require("./harness");

(async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "agentgo-popup-ext-"));
  fs.writeFileSync(path.join(dir, "manifest.json"), JSON.stringify({
    manifest_version: 3, name: "Popup Test", version: "1.0", default_locale: "en",
    action: { default_popup: "popup/popup.html" }, options_page: "options.html",
    content_scripts: [{ matches: ["<all_urls>"], js: ["cs.js"] }],
  }));
  fs.mkdirSync(path.join(dir, "_locales", "en"), { recursive: true });
  fs.mkdirSync(path.join(dir, "popup"));
  fs.writeFileSync(path.join(dir, "_locales", "en", "messages.json"), JSON.stringify({ title: { message: "Pick a colour" } }));
  fs.writeFileSync(path.join(dir, "cs.js"), "chrome.storage.local.get({ color: 'none' }, r => { document.documentElement.dataset.color = r.color; });");
  fs.writeFileSync(path.join(dir, "popup", "popup.html"), "<!doctype html><html><head><meta charset=utf-8><link rel=stylesheet href=popup.css></head><body><h1 id=t></h1><p id=tab></p><p id=now></p><button id=go>Green</button><script src=popup.js></script></body></html>");
  fs.writeFileSync(path.join(dir, "popup", "popup.css"), "body{width:300px}");
  fs.writeFileSync(path.join(dir, "popup", "popup.js"), `
    document.getElementById('t').textContent = chrome.i18n.getMessage('title');
    chrome.tabs.query({ active: true, currentWindow: true }, tabs => { document.getElementById('tab').textContent = tabs[0] && tabs[0].url; });
    chrome.storage.local.get({ color: 'none' }, r => { document.getElementById('now').textContent = r.color; });
    document.getElementById('go').onclick = () => chrome.storage.local.set({ color: 'green' }, () => { document.getElementById('now').textContent = 'green'; });`);
  fs.writeFileSync(path.join(dir, "options.html"), "<!doctype html><title>Options</title><p id=o>options page</p>");
  const site = http.createServer((_, r) => r.writeHead(200, { "Content-Type": "text/html" }).end("<title>Shop</title><p>shop page</p>"));
  await new Promise((r) => site.listen(0, "127.0.0.1", r));
  const url = `http://127.0.0.1:${site.address().port}/`;
  const port = await reservePort();
  const browser = launch({ args: ["--url", url, "--mode", "full", "--allow-evaluate"], debugPort: port });
  const operator = (method, params) => cdpEval(port, "chrome.html", `callBridge(${JSON.stringify(method)}, ${JSON.stringify(params)})`);
  let failed = false;
  try {
    await delay(2000);
    const added = await operator("extension.install", { path: dir });
    assert.equal(added.popup, true);
    assert.equal(added.options, true);
    await rejects(browser.call("extension.open_popup", { id: added.id }), "policy.denied");

    // The person clicks the toolbar button: the pop-up opens with its own UI.
    const opened = await operator("extension.open_popup", { id: added.id });
    assert.equal(opened.url, `agentgo-ext://${added.id}/popup/popup.html`);
    const popup = (expr) => cdpEval(port, `agentgo-ext://${added.id}/popup`, expr);
    let shown = null;
    for (let i = 0; i < 40 && !(shown && shown.t); i += 1) {
      shown = await popup(`({ t: document.getElementById('t')?.textContent, tab: document.getElementById('tab')?.textContent, now: document.getElementById('now')?.textContent, width: getComputedStyle(document.body).width })`).catch(() => null);
      await delay(150);
    }
    assert.deepEqual(shown, { t: "Pick a colour", tab: url, now: "none", width: "300px" }, JSON.stringify(shown));

    // A setting changed in the pop-up reaches the extension's content script.
    await popup(`document.getElementById('go').click()`);
    let saved;
    for (let i = 0; i < 30; i += 1) {
      saved = await operator("extension.storage", { id: added.id });
      if (saved.storage.color === "green") break;
      await delay(100);
    }
    assert.equal(saved.storage.color, "green", "the pop-up's chrome.storage is saved");
    await browser.call("navigate.url", { url });
    await delay(800);
    const color = await browser.call("runtime.evaluate", { expression: "document.documentElement.dataset.color" });
    assert.equal(color.value, "green", "the content script sees the pop-up's setting");

    // The pop-up is extension code: AgentGo's trusted bridge refuses it.
    const bridge = await popup(`fetch('agentgo://shell/__bridge?method=settings.get&params=%7B%7D').then(r => r.status).catch(e => 'blocked: ' + e.message)`);
    assert.notEqual(bridge, 200, `the pop-up reached the bridge (${bridge})`);
    // A website cannot read an extension's files.
    const probe = await browser.call("runtime.evaluate", { expression: `fetch('agentgo-ext://${added.id}/popup/popup.js').then(r => r.status).catch(e => 'blocked')`, awaitPromise: true });
    assert.notEqual(probe.value, 200, "a website read the extension's files");
    // ...nor open its pages: only its toolbar button (or Options) does.
    await browser.call("runtime.evaluate", { expression: `location.href = 'agentgo-ext://${added.id}/options.html'; true` });
    await delay(1000);
    const stayed = await browser.call("tabs.status");
    assert(!String(stayed.url).startsWith("agentgo-ext://"), `a website opened an extension page (${stayed.url})`);

    // Options opens the options page.
    const options = await operator("extension.open_popup", { id: added.id, options: true });
    assert.equal(options.url, `agentgo-ext://${added.id}/options.html`);
    console.log("extension_popup_e2e: ok");
  } catch (error) {
    failed = true;
    console.error(error);
    console.error(browser.stderr().slice(-3000));
  } finally {
    await browser.close();
    site.close();
    fs.rmSync(dir, { recursive: true, force: true });
    process.exit(failed ? 1 : 0);
  }
})();
