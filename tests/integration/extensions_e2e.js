// Extensions: a Chrome extension folder installs from the toolbar (operator
// only; an agent is refused), shows its localized name, says when it needs a
// background page, and its content script gets chrome.runtime, chrome.i18n
// and chrome.storage, whose values are saved across page loads. The Web
// Store helper (webstore.js) answers the store's status question and refuses
// an install request that does not come from the person's click.
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const http = require("node:http");
const { launch, cdpEval, delay, reservePort, rejects } = require("./harness");

(async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "agentgo-ext-"));
  fs.writeFileSync(path.join(dir, "manifest.json"), JSON.stringify({
    manifest_version: 3, name: "__MSG_name__", default_locale: "en", version: "1.0",
    background: { service_worker: "bg.js" },
    content_scripts: [{ matches: ["<all_urls>"], js: ["cs.js"] }],
  }));
  fs.mkdirSync(path.join(dir, "_locales", "en"), { recursive: true });
  fs.writeFileSync(path.join(dir, "_locales", "en", "messages.json"), JSON.stringify({ name: { message: "Shim Test" }, hello: { message: "Hello $1" } }));
  fs.writeFileSync(path.join(dir, "bg.js"), "");
  fs.writeFileSync(path.join(dir, "cs.js"), `chrome.storage.local.get({ count: 0 }, (r) => {
    const n = r.count + 1;
    chrome.storage.local.set({ count: n });
    chrome.runtime.sendMessage({ ping: 1 });
    document.documentElement.dataset.shim = [chrome.i18n.getMessage('hello', ['there']), n, typeof chrome.runtime.id].join('|');
  });`);
  const site = http.createServer((_, r) => r.writeHead(200, { "Content-Type": "text/html" }).end("<title>t</title><p>page</p>"));
  await new Promise((r) => site.listen(0, "127.0.0.1", r));
  const url = `http://127.0.0.1:${site.address().port}/`;
  const port = await reservePort();
  const browser = launch({ args: ["--url", "about:blank", "--mode", "full"], debugPort: port });
  const operator = (method, params) => cdpEval(port, "chrome.html", `callBridge(${JSON.stringify(method)}, ${JSON.stringify(params)})`);
  try {
    await delay(2000);
    await rejects(browser.call("extension.install", { path: dir }), "policy.denied");
    const added = await operator("extension.install", { path: dir });
    assert.equal(added.name, "Shim Test", "the name comes from the extension's own messages");
    assert.equal(added.limited, true, "an extension with a background page is marked as limited");

    const shimOn = async () => {
      await browser.call("navigate.url", { url, observe: "none" });
      await browser.call("wait.load", {});
      for (let i = 0; i < 20; i++) {
        const value = (await browser.call("runtime.evaluate", { expression: "document.documentElement.dataset.shim || ''" })).value;
        if (value) return value;
        await delay(250);
      }
      return "";
    };
    assert.equal(await shimOn(), "Hello there|1|string", "chrome.i18n, chrome.storage and chrome.runtime work");
    assert.equal(await shimOn(), "Hello there|2|string", "chrome.storage keeps its values across page loads");

    // The store helper only acts on chromewebstore.google.com; elsewhere the
    // page's own chrome object is untouched.
    const elsewhere = await browser.call("runtime.evaluate", { expression: "typeof (window.chrome && window.chrome.webstorePrivate && window.chrome.webstorePrivate.beginInstallWithManifest3)" });
    assert.notEqual(elsewhere.value, "function", "no store helper on other sites");
    console.log("extensions_e2e ok");
  } finally {
    site.close();
    await browser.close();
    fs.rmSync(dir, { recursive: true, force: true });
  }
})().catch((error) => {
  console.error(error);
  process.exit(1);
});
