// Dev helper: launch the runtime, run optional page-side steps, and save CDP
// screenshots of every AgentGo surface. Usage:
//   node scripts/dev/shot.js <outDir> [--url URL] [--eval "target-substring::js"]... [--wait ms] [--size WxH]
const { spawn } = require("node:child_process");
const fs = require("node:fs");
const net = require("node:net");
const os = require("node:os");
const path = require("node:path");

const repo = path.resolve(__dirname, "../..");
const exe = path.join(repo, "target", "debug", "agentgo.exe");
const args = process.argv.slice(2);
const outDir = path.resolve(args.shift() || "shots");
const opt = { evals: [], wait: 2500, url: null, size: null };
for (let i = 0; i < args.length; i += 1) {
  if (args[i] === "--url") opt.url = args[++i];
  else if (args[i] === "--eval") opt.evals.push(args[++i]);
  else if (args[i] === "--wait") opt.wait = Number(args[++i]);
  else if (args[i] === "--size") opt.size = args[++i].split("x").map(Number);
}
const delay = (ms) => new Promise((r) => setTimeout(r, ms));
const reservePort = () => new Promise((resolve, reject) => {
  const s = net.createServer();
  s.once("error", reject);
  s.listen(0, "127.0.0.1", () => { const p = s.address().port; s.close(() => resolve(p)); });
});
async function cdp(target) {
  const ws = new WebSocket(target.webSocketDebuggerUrl);
  const pending = new Map(); let id = 1;
  await new Promise((res, rej) => { ws.addEventListener("open", res, { once: true }); ws.addEventListener("error", rej, { once: true }); });
  ws.addEventListener("message", (e) => { const m = JSON.parse(e.data); const w = pending.get(m.id); if (w) { pending.delete(m.id); m.error ? w.reject(new Error(m.error.message)) : w.resolve(m.result); } });
  return { send: (method, params = {}) => new Promise((resolve, reject) => { pending.set(id, { resolve, reject }); ws.send(JSON.stringify({ id: id++, method, params })); }), close: () => ws.close() };
}
(async () => {
  fs.mkdirSync(outDir, { recursive: true });
  const port = await reservePort();
  const dataRoot = fs.mkdtempSync(path.join(os.tmpdir(), "agentgo-shot-"));
  const runArgs = ["run", "--stdio"]; if (opt.url) runArgs.push("--url", opt.url);
  const child = spawn(exe, runArgs, { cwd: repo, env: { ...process.env, AGENTGO_DATA_ROOT: dataRoot, AGENTGO_REMOTE_DEBUGGING_PORT: String(port), AGENTGO_UNSAFE_DEVTOOLS: "1" }, stdio: ["pipe", "pipe", "pipe"] });
  let stderr = ""; child.stderr.on("data", (c) => { stderr += c; }); child.stdout.on("data", (c) => { process.stdout.write(c); });
  const send = (id, method, params = {}) => child.stdin.write(JSON.stringify({ v: 1, id, method, params }) + "\n");
  const targets = async () => { try { return await (await fetch(`http://127.0.0.1:${port}/json/list`)).json(); } catch { return []; } };
  try {
    for (let i = 0; i < 100 && (await targets()).length < 3; i += 1) await delay(100);
    if (opt.size) send(900, "window.resize", { width: opt.size[0], height: opt.size[1] });
    await delay(opt.wait);
    for (const spec of opt.evals) {
      const [needle, ...rest] = spec.split("::"); const js = rest.join("::");
      if (needle === "sleep") { await delay(Number(js)); continue; }
      if (needle === "rpc") { const [m, p] = js.split("|"); send(901 + Math.floor(Math.random() * 1000), m, p ? JSON.parse(p) : {}); await delay(300); continue; }
      const t = (await targets()).find((x) => x.url.includes(needle));
      if (!t) { console.log("NO TARGET", needle); continue; }
      const c = await cdp(t);
      const r = await c.send("Runtime.evaluate", { expression: js, returnByValue: true, awaitPromise: true });
      console.log("EVAL", needle, JSON.stringify(r.result?.value ?? r.exceptionDetails?.exception?.description ?? null));
      c.close();
    }
    let n = 0;
    for (const t of await targets()) {
      if (t.type !== "page") continue;
      const c = await cdp(t);
      const shot = await Promise.race([c.send("Page.captureScreenshot", { format: "png" }), delay(4000).then(() => null)]);
      if (!shot) { console.log("SKIP (hidden)", t.url); c.close(); continue; }
      const name = `${n++}-${t.url.replace(/^[a-z]+:\/\//, "").replace(/[^a-z0-9.]+/gi, "_").slice(0, 50)}.png`;
      fs.writeFileSync(path.join(outDir, name), Buffer.from(shot.data, "base64"));
      console.log("SHOT", name, t.url);
      c.close();
    }
  } finally {
    send(999, "browser.shutdown");
    await Promise.race([new Promise((r) => child.once("exit", r)), delay(8000)]);
    child.kill();
    fs.writeFileSync(path.join(outDir, "stderr.log"), stderr);
    fs.rmSync(dataRoot, { recursive: true, force: true });
  }
})().catch((e) => { console.error(e); process.exit(1); });
