// Shared helpers for integration tests: start the real browser with an
// isolated data root, speak the protocol over stdio or the named pipe, act as
// the human operator through the trusted chrome, and serve local fixtures.
const { spawn } = require("node:child_process");
const fs = require("node:fs");
const http = require("node:http");
const net = require("node:net");
const os = require("node:os");
const path = require("node:path");
const { pathToFileURL } = require("node:url");

const repo = path.resolve(__dirname, "../..");
// AGENTGO_EXE points the tests at another build, for example a packaged copy.
const executable = process.env.AGENTGO_EXE || path.join(repo, "target", "debug", "agentgo.exe");
const pagesDir = path.join(repo, "tests", "fixtures", "pages");
const delay = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
// Agents may open file:// fixtures only in a folder the operator granted; every
// browser a test starts (directly or through the MCP server) gets this one.
process.env.AGENTGO_UPLOAD_DIRS = pagesDir;
const fixture = (name) => pathToFileURL(path.join(pagesDir, name)).href;

function reservePort() {
  return new Promise((resolve, reject) => {
    const server = net.createServer();
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const { port } = server.address();
      server.close(() => resolve(port));
    });
  });
}

// Newline-delimited JSON client over any duplex stream pair.
function protocolClient(write, onStderr) {
  let buffer = "";
  let nextId = 1;
  const waiters = new Map();
  const events = [];
  function feed(chunk) {
    buffer += chunk;
    const lines = buffer.split("\n");
    buffer = lines.pop();
    for (const line of lines) {
      if (!line.trim()) continue;
      const message = JSON.parse(line); // the stream must be pure protocol JSON
      if (message.event) { events.push(message); continue; }
      const waiter = waiters.get(message.id);
      if (waiter) { waiters.delete(message.id); waiter(message); }
    }
  }
  function raw(method, params = {}, timeoutMs = 30_000, { unref = false } = {}) {
    const id = nextId++;
    try { write(JSON.stringify({ v: 1, id, method, params }) + "\n"); } catch (error) {
      return Promise.resolve({ ok: false, error: { code: "transport.closed", message: error.message } });
    }
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        waiters.delete(id);
        reject(new Error(`timed out waiting for ${method}\n${onStderr ? onStderr().slice(-2000) : ""}`));
      }, timeoutMs);
      // A caller that races the reply against something else must not be kept
      // alive by this timer once the race is decided.
      if (unref) timer.unref();
      waiters.set(id, (message) => { clearTimeout(timer); resolve(message); });
    });
  }
  async function call(method, params = {}, timeoutMs) {
    const message = await raw(method, params, timeoutMs);
    if (!message.ok) {
      const error = new Error(`${method}: ${message.error.code}: ${message.error.message}`);
      error.code = message.error.code;
      error.data = message.error.data;
      throw error;
    }
    return message.result;
  }
  // The other side is gone: answer everything still waiting, now and later.
  function abort(reason) {
    write = () => { throw new Error(reason); };
    for (const waiter of waiters.values()) waiter({ ok: false, error: { code: "transport.closed", message: reason } });
    waiters.clear();
  }
  return { feed, raw, call, events, abort };
}

// A test browser opens a real window. Unless the run is on a private desktop
// (`agentgo-tool quiet-run`, which `scripts/dev/run-integration.js` always
// uses) that window would pop up in front of whoever is working. Refuse to
// start rather than interrupt them; AGENTGO_TEST_VISIBLE=1 is the explicit
// opt-in for watching a test on purpose.
function requireQuiet() {
  if (process.env.AGENTGO_QUIET_DESKTOP === "1" || process.env.AGENTGO_TEST_VISIBLE === "1") return;
  throw new Error(
    "refusing to open a browser window on your desktop. Run the test with\n" +
    "  node scripts/dev/run-integration.js <test name>\n" +
    "or  target\\debug\\agentgo-tool.exe quiet-run -- node <test file>\n" +
    "(set AGENTGO_TEST_VISIBLE=1 only if you want to watch it)."
  );
}

function launch({ args = [], env = {}, dataRoot, stdio = true, debugPort } = {}) {
  requireQuiet();
  const root = dataRoot || fs.mkdtempSync(path.join(os.tmpdir(), "agentgo-test-"));
  const child = spawn(executable, ["run", ...(stdio ? ["--stdio"] : []), ...args], {
    cwd: repo,
    env: {
      ...process.env,
      AGENTGO_DATA_ROOT: root,
      // No first-run wizard and no permission prompts: tests answer by policy.
      AGENTGO_SKIP_SETUP: "1",
      AGENTGO_NO_PROMPTS: "1",
      // Quiet runs never raise Windows notifications or flash the taskbar;
      // a run on the person's desktop shows them, as the product does.
      ...(process.env.AGENTGO_TEST_VISIBLE === "1" ? {} : { AGENTGO_NO_NOTIFY: "1" }),
      ...(debugPort ? { AGENTGO_REMOTE_DEBUGGING_PORT: String(debugPort), AGENTGO_UNSAFE_DEVTOOLS: "1" } : {}),
      ...env,
    },
    stdio: ["pipe", "pipe", "pipe"],
    windowsHide: true,
  });
  let stderr = "";
  const client = protocolClient((line) => child.stdin.write(line), () => stderr);
  child.stdout.setEncoding("utf8");
  child.stderr.setEncoding("utf8");
  child.stderr.on("data", (chunk) => { stderr += chunk; });
  child.stdout.on("data", (chunk) => { if (stdio) client.feed(chunk); else stderr += chunk; });
  const exited = new Promise((resolve) => child.once("exit", resolve));
  // A crashed browser must fail the test at once with its exit code, not
  // look like a command that timed out.
  child.once("exit", (code) => client.abort(`the browser exited (code ${code === null ? "none" : "0x" + (code >>> 0).toString(16)})`));
  child.stdin.on("error", () => {});

  let closing = null;
  function close() { return closing ??= doClose(); }
  async function doClose() {
    // The browser may exit before it flushes the shutdown reply, so the
    // process exit counts as the answer too.
    if (stdio) { try { await Promise.race([client.raw("browser.shutdown", {}, 10_000, { unref: true }), exited]); } catch (_) {} }
    else child.kill();
    const code = await Promise.race([exited, delay(15_000).then(() => "timeout")]);
    if (code === "timeout") child.kill();
    if (!dataRoot) fs.rmSync(root, { recursive: true, force: true });
    return code;
  }
  return { child, ...client, close, root, stderr: () => stderr, exited, debugPort };
}

// Connects to an instance over its named pipe and authenticates.
async function attach(root, instanceId, { client = "test-agent", token } = {}) {
  const secret = token ?? fs.readFileSync(path.join(root, "instances", instanceId, "token"), "utf8");
  const socket = net.connect(`\\\\.\\pipe\\agentgo-${instanceId}`);
  await new Promise((resolve, reject) => { socket.once("connect", resolve); socket.once("error", reject); });
  socket.setEncoding("utf8");
  const proto = protocolClient((line) => socket.write(line));
  socket.on("data", proto.feed);
  const closed = new Promise((resolve) => socket.once("close", resolve));
  const hello = await proto.raw("session.hello", { token: secret, client });
  return { ...proto, hello, socket, closed, end: () => socket.end() };
}

// DevTools helpers used only by tests to play the human operator: they drive
// the trusted chrome document, which is the one place operator commands are
// accepted from.
async function cdpTarget(port, match) {
  for (let attempt = 0; attempt < 100; attempt += 1) {
    try {
      const targets = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
      const target = targets.find((t) => t.url.includes(match));
      if (target) return target;
    } catch (_) {}
    await delay(100);
  }
  throw new Error(`no DevTools target matching ${match}`);
}

async function cdpEval(port, match, expression) {
  const target = await cdpTarget(port, match);
  const socket = new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => {
    socket.addEventListener("open", resolve, { once: true });
    socket.addEventListener("error", reject, { once: true });
  });
  const result = await new Promise((resolve) => {
    socket.addEventListener("message", (event) => {
      const message = JSON.parse(event.data);
      if (message.id === 1) resolve(message.result);
    });
    socket.send(JSON.stringify({ id: 1, method: "Runtime.evaluate", params: { expression, returnByValue: true, awaitPromise: true } }));
  });
  socket.close();
  if (result.exceptionDetails) throw new Error(result.exceptionDetails.exception?.description || "evaluate failed");
  return result.result.value;
}

// Issues a command exactly as the browser chrome would for the human.
async function operator(port, method, params = {}) {
  const url = `agentgo://shell/__bridge?method=${method}&params=${encodeURIComponent(JSON.stringify(params))}`;
  const body = await cdpEval(port, "chrome.html", `fetch(${JSON.stringify(url)}).then(r => r.json())`);
  if (!body.ok) {
    const error = new Error(`${method}: ${body.error.code}: ${body.error.message}`);
    error.code = body.error.code;
    throw error;
  }
  return body.result;
}

// Serves tests/fixtures/pages over HTTP (cookies and network need an origin).
function serve() {
  return new Promise((resolve) => {
    const types = { ".html": "text/html; charset=utf-8", ".txt": "text/plain", ".json": "application/json" };
    const server = http.createServer((request, response) => {
      const name = path.basename(new URL(request.url, "http://x").pathname) || "fixture.html";
      const file = path.join(pagesDir, name);
      if (!fs.existsSync(file)) { response.writeHead(404).end("missing"); return; }
      response.writeHead(200, { "Content-Type": types[path.extname(name)] || "application/octet-stream" });
      response.end(fs.readFileSync(file));
    });
    server.listen(0, "127.0.0.1", () => resolve({
      url: (name) => `http://127.0.0.1:${server.address().port}/${name}`,
      close: () => server.close(),
    }));
  });
}

// Asserts that a call fails with the given typed error code.
async function rejects(promise, code) {
  const error = await promise.then(() => null, (e) => e);
  if (!error) throw new Error(`expected ${code}, but the call succeeded`);
  if (error.code !== code) throw new Error(`expected ${code}, got ${error.message}`);
  return error;
}

// Finds the numeric reference of the first snapshot line containing `needle`.
function refOf(page, needle) {
  const line = page.split("\n").find((l) => l.includes(needle));
  if (!line) throw new Error(`no line containing ${needle} in:\n${page}`);
  return Number(line.match(/^\[(\d+)\]/)[1]);
}

module.exports = { launch, attach, operator, cdpEval, serve, reservePort, delay, fixture, repo, executable, rejects, refOf };
