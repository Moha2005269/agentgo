// Searches the Chrome Web Store's own scripts (as loaded in AgentGo) for how
// it decides the browser is Chrome. Prints short excerpts around each hit.
//   node scripts/dev/webstore-scan.js
const { launch, reservePort, delay, cdpTarget } = require("../../tests/integration/harness");

(async () => {
  const port = await reservePort();
  const browser = launch({ args: ["--url", "https://example.com/"], stdio: false, debugPort: port });
  try {
    await delay(6000);
    const target = await cdpTarget(port, "example.com");
    const socket = new WebSocket(target.webSocketDebuggerUrl);
    await new Promise((resolve) => socket.addEventListener("open", resolve, { once: true }));
    let next = 1;
    const waiting = new Map();
    const scripts = [];
    let shownHeaders = false;
    socket.addEventListener("message", (event) => {
      const m = JSON.parse(event.data);
      if (m.id && waiting.has(m.id)) { waiting.get(m.id)(m); waiting.delete(m.id); }
      else if (m.method === "Debugger.scriptParsed" && /chromewebstore|gstatic|google/.test(m.params.url)) scripts.push(m.params);
      else if (m.method === "Network.requestWillBeSentExtraInfo" && !shownHeaders) { const h = m.params.headers; if (h[":authority"] === "chromewebstore.google.com" || /chromewebstore/.test(h["referer"] || h[":path"] || "")) { shownHeaders = true; console.log("HEADERS", JSON.stringify(Object.fromEntries(Object.entries(h).filter(([k]) => /sec-ch|user-agent|:path|:authority/i.test(k))))); } }
    });
    const send = (method, params = {}) => new Promise((resolve) => { const i = next++; waiting.set(i, resolve); socket.send(JSON.stringify({ id: i, method, params })); });
    await send("Network.enable");
    if (process.argv[2] !== "scan") { await send("Page.navigate", { url: "https://chromewebstore.google.com/detail/ddkjiahejlhfcafbddmgiahcphecmpfh" }); await delay(8000); const html = await send("Runtime.evaluate", { expression: "document.documentElement.outerHTML.length + \" \" + (document.documentElement.outerHTML.match(/.{0,200}(Switch to Chrome|Install Chrome).{0,200}/) || [\"\"])[0]", returnByValue: true }); console.log("HTML", html.result.result.value); socket.close(); return; }
    await send("Debugger.enable");
    await send("Page.navigate", { url: "https://chromewebstore.google.com/detail/ddkjiahejlhfcafbddmgiahcphecmpfh" });
    await delay(12000);
    const needles = ["userAgentData", "webstorePrivate", "Google Chrome", "getBrands", "isChrome", "brands"];
    for (const script of scripts) {
      for (const needle of needles) {
        const found = await send("Debugger.searchInContent", { scriptId: script.scriptId, query: needle });
        for (const hit of (found.result?.result || []).slice(0, 4)) {
          const line = hit.lineContent;
          const at = line.indexOf(needle);
          console.log(`[${needle}] ${script.url.slice(-60)}: ${line.slice(Math.max(0, at - 160), at + 200).replace(/\s+/g, " ")}`);
        }
      }
    }
    socket.close();
  } finally {
    await browser.close().catch(() => {});
  }
})();
