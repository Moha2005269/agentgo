// Security boundaries: agents cannot read local files, web pages cannot reach
// AgentGo's own documents, the local SDK server refuses rebinding hosts and
// foreign origins, and an unsigned update manifest is ignored.
const assert = require("node:assert/strict");
const http = require("node:http");
const { launch, operator, cdpEval, reservePort, delay, rejects } = require("./harness");

function raw(port, path, headers) {
  return new Promise((resolve, reject) => {
    const request = http.request({ host: "127.0.0.1", port, path, method: "GET", headers, setHost: false }, (response) => {
      response.resume();
      response.on("end", () => resolve(response.statusCode));
    });
    request.on("error", reject);
    request.end();
  });
}

(async () => {
  const site = http.createServer((request, response) => {
    if (request.url === "/update.json") {
      response.writeHead(200, { "Content-Type": "application/json" }).end(JSON.stringify({ version: "9.9.9", url: "https://example.invalid/x.zip", sha256: "00" }));
    } else {
      response.writeHead(200, { "Content-Type": "text/html" }).end("<title>Plain</title><p>hello</p>");
    }
  });
  await new Promise((resolve) => site.listen(0, "127.0.0.1", resolve));
  const base = `http://127.0.0.1:${site.address().port}`;
  const port = await reservePort();
  const browser = launch({ args: ["--url", "about:blank"], debugPort: port });
  try {
    // 1. An agent cannot open local files.
    await rejects(browser.call("navigate.url", { url: "file:///C:/Windows/win.ini" }), "policy.denied");
    await rejects(browser.call("tabs.create", { url: "FILE:///C:/Windows/win.ini" }), "policy.denied");

    // 2. Nor AgentGo's own documents.
    const refused = await browser.call("tabs.navigate", { url: "agentgo://shell/chrome.html" }).then(() => null, (e) => e);
    assert.ok(refused, "agentgo://shell/chrome.html must be refused for a content tab");

    // 3. A page cannot navigate itself into them either.
    await browser.call("navigate.url", { url: `${base}/page` });
    await cdpEval(port, base, "setTimeout(() => { location.href = 'agentgo://shell/tools.html'; }, 0); 1");
    await delay(1500);
    const where = await cdpEval(port, base, "location.href");
    assert.ok(where.startsWith(base), `page stayed on the web origin (was ${where})`);

    // 4. The local SDK server refuses rebinding hosts and foreign origins.
    const on = await operator(port, "sdk.http", { on: true });
    assert.equal(await raw(on.port, "/v1/health", { Host: `127.0.0.1:${on.port}` }), 200);
    assert.equal(await raw(on.port, "/v1/health", { Host: "evil.example" }), 403);
    assert.equal(await raw(on.port, "/v1/health", { Host: "127.0.0.1.evil.example" }), 403);
    assert.equal(await raw(on.port, "/v1/health", { Host: `127.0.0.1:${on.port}`, Origin: "http://127.0.0.1.evil.example" }), 403);
    assert.equal(await raw(on.port, "/v1/health", { Host: `127.0.0.1:${on.port}`, Origin: "https://evil.example" }), 403);
    await operator(port, "sdk.http", { on: false });

    // 5. An unsigned update manifest is ignored.
    const update = await browser.call("browser.update_check", { manifest: `${base}/update.json` }).then(() => null, (e) => e);
    assert.ok(update && update.code === "policy.denied", `unsigned manifest must be denied, got ${update && update.message}`);

    const status = await browser.call("security.status", {});
    assert.equal(typeof status.rendererSandbox, "boolean");
    assert.ok(status.updates && status.agentFileAccess, "security.status reports updates and file access");
    for (let i = 0; i < 50 && !status.adBlocker?.engineReady; i += 1) {
      await new Promise((resolve) => setTimeout(resolve, 200));
      Object.assign(status, await browser.call("security.status", {}));
    }
    assert.equal(status.adBlocker.active, true, "security.status says the ad blocker is active");
    assert.ok(status.adBlocker.lists.length >= 10, "security.status lists the filter lists");
    assert.ok(status.adBlocker.youtube.hosts.includes("youtube.com"), "security.status covers YouTube");

    console.log("security_e2e ok");
  } finally {
    site.close();
    await browser.close();
  }
})().catch((error) => {
  console.error(error);
  process.exit(1);
});
