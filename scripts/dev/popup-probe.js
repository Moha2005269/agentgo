// Live look at a real Web Store extension's pop-up in AgentGo: installs it
// from the store, opens its pop-up, prints what the pop-up shows and its
// console errors, and saves a screenshot to target/integration-logs.
//   node scripts/dev/popup-probe.js [extension id]
const fs = require("node:fs");
const path = require("node:path");
const { launch, reservePort, delay, cdpTarget, cdpEval, repo } = require("../../tests/integration/harness");

const id = process.argv[2] || "eimadpbcbfnmbkopoojfekhnkhdbieeh"; // Dark Reader

(async () => {
  const port = await reservePort();
  const browser = launch({ args: ["--url", "https://example.com/", "--mode", "full"], debugPort: port });
  const operator = (method, params) => cdpEval(port, "chrome.html", `callBridge(${JSON.stringify(method)}, ${JSON.stringify(params)})`);
  try {
    await delay(3000);
    const added = await operator("extension.install", { store: id });
    console.log("installed:", added.name, "popup:", added.popup, "limited:", added.limited);
    await operator("extension.open_popup", { id: added.id });
    await delay(4000);
    const target = await cdpTarget(port, `agentgo-ext://${added.id}/`);
    const socket = new WebSocket(target.webSocketDebuggerUrl);
    await new Promise((r) => socket.addEventListener("open", r, { once: true }));
    let next = 1; const waiting = new Map(); const logs = [];
    socket.addEventListener("message", (e) => { const m = JSON.parse(e.data); if (waiting.has(m.id)) { waiting.get(m.id)(m); waiting.delete(m.id); } else if (m.method === "Runtime.exceptionThrown") logs.push("exception: " + (m.params.exceptionDetails.exception?.description || m.params.exceptionDetails.text).slice(0, 200)); else if (m.method === "Runtime.consoleAPICalled" && m.params.type === "error") logs.push("console.error: " + m.params.args.map((a) => a.value ?? a.description).join(" ").slice(0, 200)); });
    const send = (method, params = {}) => new Promise((r) => { const i = next++; waiting.set(i, r); socket.send(JSON.stringify({ id: i, method, params })); });
    await send("Runtime.enable");
    await send("Page.reload");
    await delay(4000);
    const text = (await send("Runtime.evaluate", { expression: "document.body ? document.body.innerText.slice(0, 400) : ''", returnByValue: true })).result.result.value;
    console.log("pop-up shows:", JSON.stringify(text));
    for (const line of logs.slice(0, 8)) console.log(line);
    const shot = await send("Page.captureScreenshot", { format: "png" });
    fs.writeFileSync(path.join(repo, "target", "integration-logs", `popup-${added.id}.png`), Buffer.from(shot.result.data, "base64"));
    socket.close();
  } finally {
    await browser.close();
  }
})().catch((e) => { console.error(e); process.exit(1); });
