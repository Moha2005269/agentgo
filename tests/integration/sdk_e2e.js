// The local HTTP/WebSocket transport and the SDK clients: the person turns
// the transport on; the TypeScript, Python and Rust clients call through it
// with the token; a WebSocket session gets replies and events; the plugin
// template installs as a skill.
const assert = require("node:assert/strict");
const path = require("node:path");
const { execFileSync } = require("node:child_process");
const { pathToFileURL } = require("node:url");
const { launch, operator, reservePort, rejects, repo } = require("./harness");

(async () => {
  const port = await reservePort();
  const browser = launch({ args: ["--url", "about:blank"], debugPort: port });
  try {
    await browser.call("browser.health");
    // Off until the person turns it on.
    assert.equal((await browser.call("sdk.http")).listening, false);
    await rejects(browser.call("sdk.http", { on: true }), "policy.denied");
    const on = await operator(port, "sdk.http", { on: true });
    assert.equal(on.listening, true);
    assert.match(on.websocket, /^ws:\/\/127\.0\.0\.1:\d+\/v1\/ws$/);
    const { port: httpPort, tokenFile } = on;

    // No token, a wrong token and a web page's request are refused.
    const url = `http://127.0.0.1:${httpPort}/v1/call`;
    assert.equal((await fetch(url, { method: "POST", body: "{}" })).status, 401);
    assert.equal((await fetch(url, { method: "POST", headers: { authorization: "Bearer nope" }, body: "{}" })).status, 401);
    assert.equal((await fetch(url, { method: "POST", headers: { origin: "https://evil.example" }, body: "{}" })).status, 403);

    // TypeScript client over HTTP and WebSocket.
    const { AgentGo, AgentGoError } = await import(pathToFileURL(path.join(repo, "sdk", "typescript", "agentgo.mjs")).href);
    const ag = await AgentGo.connect({ port: httpPort, tokenFile });
    assert.ok((await ag.call("browser.version")).agentgo);
    const tab = await ag.call("tabs.create", { url: "about:blank" });
    assert.equal((await ag.call("tabs.status", { tabId: tab.tabId })).tabId, tab.tabId, "the HTTP session keeps its tab lease");
    const failure = await ag.call("no.such.method").catch((e) => e);
    assert.ok(failure instanceof AgentGoError);
    const ws = await ag.socket();
    assert.ok((await ws.call("session.info")).session.startsWith("ws-"));
    const events = [];
    ws.onEvent = (event) => events.push(event);
    await ws.call("events.subscribe");
    await ws.call("tabs.create", { url: "about:blank" });
    ws.close();

    // Python and Rust clients.
    const py = JSON.parse(execFileSync("python", [path.join(repo, "sdk", "python", "agentgo.py"), String(httpPort), tokenFile, "browser.version"]).toString());
    assert.ok(py.agentgo);
    const rust = execFileSync("cargo", ["run", "--offline", "-q", "--manifest-path", path.join(repo, "sdk", "rust", "Cargo.toml"), "--example", "call", "--", String(httpPort), tokenFile, "browser.version"], { cwd: path.join(repo, "sdk", "rust") }).toString();
    assert.ok(JSON.parse(rust).agentgo);

    // The SDK commands.
    for (const lang of ["sdk.rust", "sdk.typescript", "sdk.python"]) assert.equal((await browser.call(lang)).present, true, lang);
    const plugin = await browser.call("sdk.plugin");
    assert.equal(plugin.present, true);
    const installed = await browser.call("skill.install_local", { path: path.join(plugin.path, "template") });
    assert.equal(installed.installed, "open-and-read");
    assert.equal((await browser.call("skill.test", { name: "open-and-read" })).passed, true);
    assert.equal((await browser.call("sdk.skill")).present, true);
    assert.equal((await browser.call("sdk.websocket")).listening, true);
    const test = await browser.call("sdk.test");
    assert.equal(test.passed, true, JSON.stringify(test));
    assert.ok(test.checks.some((c) => c.transport === "http" && c.ok === true));

    // Off again: the token stops working.
    await operator(port, "sdk.http", { on: false });
    await fetch(url, { method: "POST", headers: { authorization: `Bearer x` }, body: "{}" }).then(() => {}, () => {});
    assert.equal((await browser.call("sdk.http")).listening, false);
    console.log("sdk_e2e: PASS");
  } finally {
    await browser.close();
  }
})().catch((error) => {
  console.error(error);
  process.exit(1);
});
