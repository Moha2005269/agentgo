// Live probe of the Chrome Web Store install path: opens an extension's store
// page in AgentGo (no agent attached, as when a person browses), clicks its
// "Add to Chrome" button with a real mouse click through DevTools, and
// prints the page's console, what the store shows and AgentGo's log lines.
//   node scripts/dev/webstore-probe.js [extension id]
const fs = require("node:fs");
const path = require("node:path");
const { launch, reservePort, delay, cdpTarget } = require("../../tests/integration/harness");

const id = process.argv[2] || "ddkjiahejlhfcafbddmgiahcphecmpfh"; // uBlock Origin Lite

async function session(port, match) {
  const target = await cdpTarget(port, match);
  const socket = new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => { socket.addEventListener("open", resolve, { once: true }); socket.addEventListener("error", reject, { once: true }); });
  let next = 1;
  const waiting = new Map();
  const events = [];
  socket.addEventListener("message", (event) => {
    const m = JSON.parse(event.data);
    if (m.id && waiting.has(m.id)) { waiting.get(m.id)(m); waiting.delete(m.id); } else if (m.method) events.push(m);
  });
  const send = (method, params = {}) => new Promise((resolve) => { const i = next++; waiting.set(i, resolve); socket.send(JSON.stringify({ id: i, method, params })); });
  const evaluate = async (expression) => (await send("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true })).result?.result?.value;
  return { send, evaluate, events, close: () => socket.close() };
}

(async () => {
  const port = await reservePort();
  const url = `https://chromewebstore.google.com/detail/${id}`;
  const browser = launch({ args: ["--url", "https://example.com/"], stdio: false, debugPort: port });
  try {
    await delay(6000);
    const first = await session(port, "example.com");
    await first.send("Page.navigate", { url });
    first.close();
    await delay(8000);
    const page = await session(port, "chromewebstore.google.com");
    await page.send("Runtime.enable");
    await page.send("Log.enable");
    for (let i = 0; i < 30; i += 1) {
      const ready = await page.evaluate(`!![...document.querySelectorAll('button')].find(b => /add to chrome|remove from chrome|add to/i.test(b.innerText))`);
      if (ready) break;
      await delay(500);
    }
    const shim = await page.evaluate(`typeof (window.chrome && chrome.webstorePrivate && chrome.webstorePrivate.beginInstallWithManifest3)`);
    const box = await page.evaluate(`(() => { const b = [...document.querySelectorAll('button')].find(b => /add to chrome/i.test(b.innerText)); if (!b) return null; b.scrollIntoView({block:'center'}); const r = b.getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2, text: b.innerText }; })()`);
    console.log("shim:", shim, "button:", JSON.stringify(box));
    const identity = await page.evaluate(`(async () => ({ ua: navigator.userAgent, brands: navigator.userAgentData && navigator.userAgentData.brands, full: navigator.userAgentData && (await navigator.userAgentData.getHighEntropyValues(["fullVersionList"])).fullVersionList, app: typeof (window.chrome && chrome.app), runtime: typeof (window.chrome && chrome.runtime), csi: typeof (window.chrome && chrome.csi), loadTimes: typeof (window.chrome && chrome.loadTimes), vendor: navigator.vendor }))()`);
    console.log("identity:", JSON.stringify(identity));
    const check = await page.evaluate(`({ management: typeof (window.chrome && chrome.management), wsp: typeof (window.chrome && chrome.webstorePrivate), begin: typeof (window.chrome && chrome.webstorePrivate && chrome.webstorePrivate.beginInstallWithManifest3), storeCondition: !!(window.chrome && window.chrome.management && window.chrome.webstorePrivate && window.chrome.webstorePrivate.beginInstallWithManifest3), desc: String(Object.getOwnPropertyDescriptor(window.chrome, "management") && Object.getOwnPropertyDescriptor(window.chrome, "management").get), keys: Object.keys(window.chrome || {}).join(",") })`);
    console.log("check:", JSON.stringify(check));
    if (box) {
      await delay(500);
      for (const type of ["mouseMoved", "mousePressed", "mouseReleased"]) {
        await page.send("Input.dispatchMouseEvent", { type, x: box.x, y: box.y, button: "left", clickCount: 1 });
      }
      await delay(15000);
    }
    const shown = await page.evaluate(`[...document.querySelectorAll('[role=dialog],[role=alert],[aria-live]')].map(e => e.innerText.trim()).filter(Boolean).join(' | ') || document.body.innerText.match(/(Download error[^\\n]*|error[^\\n]{0,80})/i)?.[0] || ''`);
    const after = await page.evaluate(`[...document.querySelectorAll('button')].map(b => b.innerText).filter(t => /chrome/i.test(t)).join(' | ')`);
    console.log("store shows:", shown);
    console.log("buttons now:", after);
    for (const e of page.events) {
      if (e.method === "Runtime.consoleAPICalled") console.log("console", e.params.type, e.params.args.map((a) => a.value ?? a.description).join(" ").slice(0, 300));
      if (e.method === "Runtime.exceptionThrown") console.log("exception", (e.params.exceptionDetails.exception?.description || e.params.exceptionDetails.text).slice(0, 300));
      if (e.method === "Log.entryAdded") console.log("log", e.params.entry.level, e.params.entry.text.slice(0, 200));
    }
    page.close();
  } finally {
    for (const line of browser.stderr().split("\n")) if (/extension|AG-SECURITY|webstore|operator-notification/i.test(line)) console.log("agentgo:", line.slice(0, 240));
    const logs = path.join(browser.root, "logs");
    if (fs.existsSync(logs)) {
      for (const file of fs.readdirSync(logs)) {
        const text = fs.readFileSync(path.join(logs, file), "utf8");
        for (const line of text.split("\n")) if (/extension|AG-SECURITY|webstore/i.test(line)) console.log("agentgo:", line.slice(0, 240));
      }
    }
    const extDir = path.join(browser.root, "extensions");
    console.log("installed folders:", fs.existsSync(extDir) ? fs.readdirSync(extDir).join(",") : "(none)");
    await browser.close();
  }
})();
