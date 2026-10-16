// The shipped update path with the real release key: a manifest signed by the
// release key (as `package.ps1` produces) is accepted, the package downloads,
// verifies its hash and unpacks to a staging folder; a tampered signature, a
// tampered hash and a downgrade are all refused.
// Needs AGENTGO_RELEASE_ZIP (the packaged zip) and the signing key; skipped otherwise.
// @live
const assert = require("node:assert/strict");
const crypto = require("node:crypto");
const fs = require("node:fs");
const http = require("node:http");
const os = require("node:os");
const path = require("node:path");
const { launch, operator, reservePort, rejects } = require("./harness");

const zip = process.env.AGENTGO_RELEASE_ZIP || path.join(__dirname, "..", "..", "dist", "AgentGo-0.1.0-win64.zip");
const keyFile = process.env.AGENTGO_UPDATE_KEY || path.join(os.homedir(), ".agentgo-signing", "update-ed25519.pem");
if (!fs.existsSync(zip) || !fs.existsSync(keyFile)) {
  console.log("update_release_e2e skipped (no packaged zip or signing key)");
  process.exit(0);
}

(async () => {
  const bytes = fs.readFileSync(zip);
  const sha = crypto.createHash("sha256").update(bytes).digest("hex");
  const key = crypto.createPrivateKey(fs.readFileSync(keyFile));
  let manifest;
  const sign = (m) => ({ ...m, signature: crypto.sign(null, Buffer.from(`agentgo-update-v1\n${m.version}\n${m.url}\n${m.sha256.toLowerCase()}\n`), key).toString("hex") });
  const server = http.createServer((request, response) => {
    if (request.url === "/manifest.json") response.writeHead(200, { "Content-Type": "application/json" }).end(JSON.stringify(manifest));
    else response.writeHead(200, { "Content-Length": bytes.length }).end(bytes);
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const base = `http://127.0.0.1:${server.address().port}`;
  const port = await reservePort();
  const browser = launch({ args: ["--url", "about:blank"], debugPort: port });
  try {
    const url = `${base}/update.zip`;
    manifest = sign({ version: "9.9.9", url, sha256: sha, notes: "release key" });
    const check = await browser.call("browser.update_check", { manifest: `${base}/manifest.json` });
    assert.equal(check.available, true, "a release-signed manifest is accepted");

    manifest = { ...sign({ version: "9.9.9", url, sha256: sha, notes: "x" }), version: "9.9.8" };
    await rejects(browser.call("browser.update_check", { manifest: `${base}/manifest.json` }), "policy.denied");

    manifest = sign({ version: "9.9.9", url, sha256: "0".repeat(64), notes: "x" });
    await rejects(operator(port, "browser.update_stage", { manifest: `${base}/manifest.json` }), "policy.denied");

    manifest = sign({ version: "0.0.1", url, sha256: sha, notes: "downgrade" });
    const down = await browser.call("browser.update_check", { manifest: `${base}/manifest.json` }).catch((e) => e);
    assert.ok(down.available === false || down.code, "a downgrade is not offered");

    manifest = sign({ version: "9.9.9", url, sha256: sha, notes: "release key" });
    const staged = await operator(port, "browser.update_stage", { manifest: `${base}/manifest.json` });
    assert.equal(staged.staged, true);
    for (const file of ["agentgo.exe", "agentgo.dll", "libcef.dll", "agentgo-tool.exe"]) {
      assert.ok(fs.existsSync(path.join(staged.path, file)), `staged package contains ${file}`);
    }
    console.log("update_release_e2e ok");
  } finally {
    server.close();
    await browser.close();
  }
})().catch((error) => {
  console.error(error);
  process.exit(1);
});
