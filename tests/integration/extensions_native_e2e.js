// Extensions in Chromium's own engine: a folder whose manifest names its key
// gets the id Chromium gives it, runs AgentGo's content-script runner until
// the next start, and from then on runs natively: its background service
// worker answers its content script (run at document_start), and its pop-up
// opens as a real chrome-extension:// page whose chrome.tabs.query answers
// with the tab it was opened over. Agents still cannot open extension pages.
const assert = require("node:assert/strict");
const crypto = require("node:crypto");
const fs = require("node:fs");
const http = require("node:http");
const os = require("node:os");
const path = require("node:path");
const { launch, cdpEval, delay, reservePort, rejects } = require("./harness");

(async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "agentgo-extnative-"));
  const { publicKey } = crypto.generateKeyPairSync("rsa", { modulusLength: 2048 });
  const der = publicKey.export({ type: "spki", format: "der" });
  const id = [...crypto.createHash("sha256").update(der).digest().subarray(0, 16)]
    .map((b) => String.fromCharCode(97 + (b >> 4)) + String.fromCharCode(97 + (b & 15))).join("");
  fs.writeFileSync(path.join(dir, "manifest.json"), JSON.stringify({
    manifest_version: 3, name: "Native Test", version: "1.0", key: der.toString("base64"),
    background: { service_worker: "bg.js" }, permissions: ["storage"], host_permissions: ["<all_urls>"],
    action: { default_popup: "popup.html" },
    content_scripts: [{ matches: ["<all_urls>"], js: ["cs.js"], run_at: "document_start" }],
  }));
  fs.writeFileSync(path.join(dir, "bg.js"), "chrome.runtime.onMessage.addListener((m, s, reply) => reply({ from: 'background', tab: !!(s.tab && s.tab.id) }));");
  fs.writeFileSync(path.join(dir, "cs.js"), "document.documentElement.dataset.cs = 'ran'; chrome.runtime.sendMessage({ ping: 1 }, (r) => { document.documentElement.dataset.bg = r ? r.from : 'none'; });");
  fs.writeFileSync(path.join(dir, "popup.html"), "<!doctype html><title>popup</title><script src='popup.js'></script>");
  fs.writeFileSync(path.join(dir, "popup.js"), "chrome.tabs.query({ active: true, currentWindow: true }, (t) => { document.title = 'tabs:' + t.length + ':' + (t[0] ? t[0].url : ''); });");

  const site = http.createServer((_, r) => r.writeHead(200, { "Content-Type": "text/html" }).end("<title>t</title><p>page</p>"));
  await new Promise((r) => site.listen(0, "127.0.0.1", r));
  const url = `http://127.0.0.1:${site.address().port}/`;
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "agentgo-test-"));
  const args = ["--profile", "native", "--url", "about:blank", "--mode", "full"];
  const dataset = async (browser, key) => {
    for (let i = 0; i < 24; i++) {
      const value = (await browser.call("runtime.evaluate", { expression: `document.documentElement.dataset.${key} || ''` })).value;
      if (value) return value;
      await delay(250);
    }
    return "";
  };

  let port = await reservePort();
  let browser = launch({ args, dataRoot: root, debugPort: port });
  let operator = (method, params) => cdpEval(port, "chrome.html", `callBridge(${JSON.stringify(method)}, ${JSON.stringify(params)})`);
  try {
    await delay(2000);
    await rejects(browser.call("extension.install", { path: dir }), "policy.denied");
    const added = await operator("extension.install", { path: dir });
    assert.equal(added.id, id, "a folder that names its key gets Chromium's id for it");
    assert.equal(added.native, false, "not running in the engine before a restart");
    assert.equal(added.restartNeeded, true, "the panel offers a restart");
    await browser.call("navigate.url", { url, observe: "none" });
    assert.equal(await dataset(browser, "cs"), "ran", "until the restart its content script runs in AgentGo's runner");
    await browser.close();

    port = await reservePort();
    browser = launch({ args, dataRoot: root, debugPort: port });
    operator = (method, params) => cdpEval(port, "chrome.html", `callBridge(${JSON.stringify(method)}, ${JSON.stringify(params)})`);
    await delay(3000);
    const listed = (await operator("extension.list", {})).extensions.find((e) => e.id === id);
    assert.equal(listed.native, true, "after a restart the engine runs it");
    assert.equal(listed.restartNeeded, false);
    await browser.call("navigate.url", { url, observe: "none" });
    assert.equal(await dataset(browser, "bg"), "background", "the background service worker answers the content script");
    const workers = await fetch(`http://127.0.0.1:${port}/json/list`).then((r) => r.json());
    assert.ok(workers.some((t) => t.type === "service_worker" && t.url.startsWith(`chrome-extension://${id}/`)), "its service worker runs");

    const popup = await operator("extension.open_popup", { id });
    assert.ok(popup.url.startsWith(`chrome-extension://${id}/popup.html`), "the pop-up is the extension's own page");
    let title = "";
    for (let i = 0; i < 20 && !title.startsWith("tabs:"); i++) {
      await delay(250);
      const pages = await fetch(`http://127.0.0.1:${port}/json/list`).then((r) => r.json());
      title = (pages.find((t) => t.url.startsWith(`chrome-extension://${id}/popup.html`)) || {}).title || "";
    }
    assert.equal(title, `tabs:1:${url}`, "chrome.tabs.query in the pop-up answers with the tab it was opened over");
    // An agent cannot open the extension's pages: the address is not one it may navigate to.
    const tried = await browser.call("navigate.url", { url: `chrome-extension://${id}/popup.html`, observe: "none" }).catch((e) => ({ url: "", error: e.code }));
    assert.ok(!String(tried.url).startsWith("chrome-extension://"), `an agent never lands on an extension page: ${JSON.stringify(tried)}`);
    console.log("extensions_native_e2e: PASS");
  } finally {
    await browser.close();
    site.close();
  }
})().catch((error) => { console.error(error); process.exit(1); });
