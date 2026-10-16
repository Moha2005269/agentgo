// Adapter conformance: the same workflow (open page, fill a field, read the
// result, hit the purchase gate) through raw stdio, the CLI over the named
// pipe, and the MCP server. All three reach the same handlers and the same
// policy.
const assert = require("node:assert/strict");
const { spawn, execFile } = require("node:child_process");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { launch, fixture, executable, repo, refOf, rejects } = require("./harness");

const page = fixture("agent.html");

function cli(root, method, params) {
  const args = ["call", method];
  if (params) args.push(JSON.stringify(params));
  // Async on purpose: the test must keep draining the browser's pipes.
  return new Promise((resolve) => {
    execFile(executable, args, { cwd: repo, env: { ...process.env, AGENTGO_DATA_ROOT: root }, encoding: "utf8", timeout: 30_000 }, (error, stdout, stderr) => {
      resolve({ code: error ? (error.code ?? 1) : 0, out: stdout.trim(), err: stderr.trim() });
    });
  });
}

function mcp(root, extra = []) {
  const child = spawn(executable, ["mcp", ...extra], { cwd: repo, env: { ...process.env, AGENTGO_DATA_ROOT: root }, stdio: ["pipe", "pipe", "pipe"] });
  let buffer = "";
  let nextId = 1;
  const waiters = new Map();
  child.stdout.setEncoding("utf8");
  child.stdout.on("data", (chunk) => {
    buffer += chunk;
    const lines = buffer.split("\n");
    buffer = lines.pop();
    for (const line of lines) {
      if (!line.trim()) continue;
      const message = JSON.parse(line); // stdout must be pure JSON-RPC
      waiters.get(message.id)?.(message);
      waiters.delete(message.id);
    }
  });
  const request = (method, params = {}) => new Promise((resolve, reject) => {
    const id = nextId++;
    const timer = setTimeout(() => reject(new Error(`MCP ${method} timed out`)), 40_000);
    waiters.set(id, (message) => { clearTimeout(timer); resolve(message); });
    child.stdin.write(JSON.stringify({ jsonrpc: "2.0", id, method, params }) + "\n");
  });
  const tool = async (name, args = {}) => (await request("tools/call", { name, arguments: args })).result;
  const exited = new Promise((resolve) => child.once("exit", resolve));
  return { child, request, tool, exited };
}

(async () => {
  let failed = false;
  const browser = launch({ args: ["--url", "about:blank"] });
  const mcpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "agentgo-mcp-"));
  const server = mcp(mcpRoot);
  try {
    // ---- 1. raw stdio ------------------------------------------------------
    const direct = await browser.call("navigate.url", { url: page });
    await browser.call("act.fill", { ref: refOf(direct.page, "Full name"), text: "via stdio" });
    const stdioGate = await rejects(browser.call("act.click", { ref: refOf(direct.page, "Buy now") }), "approval.required");

    // ---- 2. CLI over the named pipe (a second, isolated session) -----------
    const none = await cli(fs.mkdtempSync(path.join(os.tmpdir(), "agentgo-empty-")), "tabs.list");
    assert.equal(none.code, 1);
    assert.match(none.err, /no running AgentGo browser/);

    const created = await cli(browser.root, "tabs.create", { url: page });
    assert.equal(created.code, 0, created.err);
    const tabId = JSON.parse(created.out).tabId;
    await cli(browser.root, "wait.load", { tabId });
    const view = await cli(browser.root, "observe.page", { tabId });
    assert.equal(view.code, 0, view.err);
    assert.match(view.out, /^AgentGo action fixture — file:/);
    assert.match(view.out, /\[\d+\] textbox "Full name" required/);
    const cliRef = (needle) => refOf(view.out.split("\n").slice(1).join("\n"), needle);
    const filled = await cli(browser.root, "act.fill", { tabId, ref: cliRef("Full name"), text: "via cli" });
    assert.match(JSON.parse(filled.out).field, /value="via cli"/);
    const cliGate = await cli(browser.root, "act.click", { tabId, ref: cliRef("Buy now") });
    assert.equal(cliGate.code, 1);
    assert.match(cliGate.err, /^approval\.required: /);
    // Each CLI call is its own session, so a tab leased by an earlier call's
    // session is free again; the stdio agent's tab stays protected while it
    // is connected.
    const foreign = await cli(browser.root, "observe.page", { tabId: "content-1" });
    assert.equal(foreign.code, 1);
    assert.match(foreign.err, /^unauthorized: /);
    assert.match((await cli(browser.root, "bad.method")).err, /^method.notFound: /);

    // ---- 3. MCP server with its own private browser -------------------------
    const init = await server.request("initialize", { protocolVersion: "2024-11-05", capabilities: {}, clientInfo: { name: "test", version: "0" } });
    assert.equal(init.result.serverInfo.name, "agentgo");
    server.child.stdin.write(JSON.stringify({ jsonrpc: "2.0", method: "notifications/initialized" }) + "\n");
    const listed = await server.request("tools/list");
    const names = listed.result.tools.map((tool) => tool.name);
    assert.deepEqual(names, ["browser_open", "browser_read", "browser_act", "browser_batch", "browser_wait", "browser_tabs", "browser_screenshot", "browser_wait_human", "browser_guide", "browser_account", "browser_call"]);
    const toolBytes = JSON.stringify(listed.result.tools).length;

    const opened = await server.tool("browser_open", { url: page });
    assert.equal(opened.isError, undefined);
    const outline = opened.content[0].text;
    assert.match(outline, /^AgentGo action fixture — file:/);
    const mcpRef = (needle) => refOf(outline.split("\n").slice(1).join("\n"), needle);

    const batch = await server.tool("browser_batch", {
      steps: [
        { action: "fill", ref: mcpRef("Full name"), text: "via mcp" },
        { action: "select", ref: mcpRef('"Plan"'), value: "Team" },
        { action: "check", ref: mcpRef("Send me news") },
        { action: "click", ref: mcpRef("Save draft") },
      ],
    });
    assert.equal(batch.isError, undefined, batch.content[0].text);
    const text = (await server.tool("browser_read", { view: "text" })).content[0].text;
    assert.match(text, /saved name=via mcp email= plan=team news=true/);

    const gate = await server.tool("browser_act", { action: "click", ref: mcpRef("Buy now") });
    assert.equal(gate.isError, true);
    assert.match(gate.content[0].text, /^approval\.required: .*approvalId/);
    const waited = await server.tool("browser_wait_human", { approvalId: gate.content[0].text.match(/"approvalId":"([^"]+)"/)[1], timeout: 300 });
    assert.match(waited.content[0].text, /"status":"pending"/);

    const shot = await server.tool("browser_screenshot", {});
    assert.equal(shot.content[0].type, "image", JSON.stringify(shot).slice(0, 300));
    assert(shot.content[0].data.length > 1000);
    const raw = await server.tool("browser_call", { method: "session.info" });
    assert.match(raw.content[0].text, /"session":"stdio"/);
    assert.equal((await server.tool("browser_act", { action: "explode" })).isError, true);

    // The three adapters produced the same gate from the same policy code.
    assert.equal(stdioGate.code, "approval.required");
    // The MCP browser is a different instance from the first one.
    const mcpInstance = JSON.parse(raw.content[0].text).instanceId;
    assert.notEqual(mcpInstance, (await browser.call("session.info")).instanceId);

    // Closing the host's stdin shuts the private browser down.
    server.child.stdin.end();
    assert.equal(await server.exited, 0);

    console.log(JSON.stringify({ ok: true, mcpToolDefinitionBytes: toolBytes, outlineChars: outline.length }));
  } catch (error) {
    console.error(error);
    failed = true;
    // Closing stdin lets the MCP server shut its private browser down; a
    // plain kill would leave that browser running.
    server.child.stdin.end();
    await Promise.race([server.exited, new Promise((done) => setTimeout(done, 10_000))]);
    server.child.kill();
  } finally {
    const code = await browser.close();
    fs.rmSync(mcpRoot, { recursive: true, force: true });
    process.exitCode = failed || code !== 0 ? 1 : 0;
  }
})();
