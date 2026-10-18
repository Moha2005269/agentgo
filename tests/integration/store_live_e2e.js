// @live Extensions from the Chrome Web Store, the two ways a person gets
// them: the store's own "Add to Chrome" button (a real click, while an agent
// is connected), and bringing them over when importing from Chrome (the ids
// from the profile, the packages fresh from the store). Uses Dark Reader,
// which AgentGo can run (content scripts).
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { launch, reservePort, delay, cdpTarget, cdpEval, operator } = require("./harness");

const DARK_READER = "eimadpbcbfnmbkopoojfekhnkhdbieeh";

async function click(port, match, expression) {
  const target = await cdpTarget(port, match);
  const socket = new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((resolve) => socket.addEventListener("open", resolve, { once: true }));
  let next = 1;
  const waiting = new Map();
  socket.addEventListener("message", (event) => { const m = JSON.parse(event.data); if (waiting.has(m.id)) { waiting.get(m.id)(m); waiting.delete(m.id); } });
  const send = (method, params = {}) => new Promise((resolve) => { const i = next++; waiting.set(i, resolve); socket.send(JSON.stringify({ id: i, method, params })); });
  let box = null;
  for (let i = 0; i < 40 && !box; i += 1) {
    box = (await send("Runtime.evaluate", { expression, returnByValue: true })).result?.result?.value;
    if (!box) await delay(500);
  }
  assert(box, "the store's Add to Chrome button did not appear");
  await delay(400);
  for (const type of ["mouseMoved", "mousePressed", "mouseReleased"]) {
    await send("Input.dispatchMouseEvent", { type, x: box.x, y: box.y, button: "left", clickCount: 1 });
  }
  socket.close();
}

const tools = (port, method, params = {}) => cdpEval(port, "tools.html",
  `fetch(${JSON.stringify(`agentgo://shell/__bridge?method=${method}&params=${encodeURIComponent(JSON.stringify(params))}`)}).then(r => r.json())`);

(async () => {
  // A Chrome profile that has Dark Reader installed.
  const importRoot = fs.mkdtempSync(path.join(os.tmpdir(), "agentgo-import-chrome-"));
  const profile = path.join(importRoot, "Google", "Chrome", "User Data", "Default");
  const version = path.join(profile, "Extensions", DARK_READER, "4.9.110_0");
  fs.mkdirSync(version, { recursive: true });
  fs.writeFileSync(path.join(version, "manifest.json"), JSON.stringify({ name: "Dark Reader", manifest_version: 3 }));
  fs.writeFileSync(path.join(profile, "Bookmarks"), JSON.stringify({ roots: { bookmark_bar: { children: [] }, other: { children: [] } } }));
  const port = await reservePort();
  const browser = launch({ args: ["--url", "https://example.com/", "--mode", "full"], debugPort: port, env: { AGENTGO_IMPORT_ROOT: importRoot } });
  let failed = false;
  try {
    // 1. The store's own button, clicked by the person (an agent is connected).
    await browser.call("navigate.url", { url: `https://chromewebstore.google.com/detail/${DARK_READER}` }, 60_000);
    await click(port, "chromewebstore.google.com", `(() => { const b = [...document.querySelectorAll('button')].find(b => /add to chrome/i.test(b.innerText)); if (!b) return null; b.scrollIntoView({block: 'center'}); const r = b.getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 }; })()`);
    let listed = [];
    for (let i = 0; i < 60; i += 1) {
      listed = (await browser.call("extension.list")).extensions || [];
      if (listed.some((e) => e.id === DARK_READER)) break;
      await delay(1000);
    }
    assert(listed.some((e) => e.id === DARK_READER), `the store button did not install Dark Reader: ${JSON.stringify(listed)}`);
    console.log("store_live_e2e: store button installed", listed.find((e) => e.id === DARK_READER).name);

    // 2. Brought over by the import: remove it, then import from "Chrome".
    await operator(port, "extension.uninstall", { id: DARK_READER });
    assert(!((await browser.call("extension.list")).extensions || []).some((e) => e.id === DARK_READER));
    await operator(port, "ui.open_tool", { tool: "settings" });
    await delay(800);
    const scan = await tools(port, "ui.import_scan");
    const chrome = scan.result.profiles.find((p) => p.browser === "Chrome");
    assert(chrome, `the fixture Chrome profile was not found: ${JSON.stringify(scan)}`);
    assert.equal(chrome.counts.extensions, 1);
    const run = await tools(port, "ui.import_run", { browser: "Chrome", id: chrome.id, options: { extensions: true } });
    assert(run.ok, JSON.stringify(run));
    assert.deepEqual(run.result.extensions, ["Dark Reader"], JSON.stringify(run.result));
    assert(((await browser.call("extension.list")).extensions || []).some((e) => e.id === DARK_READER), "the import did not install Dark Reader");
    console.log("store_live_e2e: import brought over", run.result.extensions.join(", "));
    console.log("store_live_e2e: ok");
  } catch (error) {
    failed = true;
    console.error(error);
    console.error(browser.stderr().slice(-3000));
  } finally {
    await browser.close();
    fs.rmSync(importRoot, { recursive: true, force: true });
    process.exit(failed ? 1 : 0);
  }
})();
