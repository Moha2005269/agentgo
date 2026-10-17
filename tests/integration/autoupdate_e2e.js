// Automatic updates: shortly after start the browser reads the release
// manifest, checks its Ed25519 signature and, with automatic updates off,
// shows "Update to <version>" in the toolbar. A manifest signed by another
// key is ignored. (Staging and swapping a real package: update_release_e2e.)
const assert = require("node:assert/strict");
const crypto = require("node:crypto");
const fs = require("node:fs");
const http = require("node:http");
const os = require("node:os");
const path = require("node:path");
const { launch, cdpEval, delay, reservePort } = require("./harness");

const rawHex = (key) => Buffer.from(key.export({ format: "jwk" }).x, "base64url").toString("hex");
const signed = (manifest, privateKey) => ({
  ...manifest,
  signature: crypto.sign(null, Buffer.from(`agentgo-update-v1\n${manifest.version}\n${manifest.url}\n${manifest.sha256}\n`), privateKey).toString("hex"),
});

(async () => {
  const release = crypto.generateKeyPairSync("ed25519");
  const stranger = crypto.generateKeyPairSync("ed25519");
  const base = { version: "9.9.9", url: "http://127.0.0.1/never-downloaded.zip", sha256: "0".repeat(64), notes: "test release" };
  const manifests = { good: signed(base, release.privateKey), forged: signed(base, stranger.privateKey) };
  const site = http.createServer((req, res) => {
    const which = req.url.includes("forged") ? "forged" : "good";
    res.writeHead(200, { "Content-Type": "application/json" }).end(JSON.stringify(manifests[which]));
  });
  await new Promise((r) => site.listen(0, "127.0.0.1", r));
  const address = (which) => `http://127.0.0.1:${site.address().port}/${which}/update-manifest.json`;

  const run = async (which) => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), "agentgo-update-"));
    fs.mkdirSync(path.join(root, "config"), { recursive: true });
    fs.writeFileSync(path.join(root, "config", "settings.json"), JSON.stringify({ schema_version: 1, autoUpdate: false, updateManifest: address(which) }));
    const port = await reservePort();
    const browser = launch({
      args: ["--url", "about:blank"], dataRoot: root, debugPort: port,
      env: { AGENTGO_UPDATE_TEST: "1", AGENTGO_UPDATE_DELAY_MS: "1500", AGENTGO_UPDATE_PUBKEY: rawHex(release.publicKey) },
    });
    try {
      let status = {};
      for (let i = 0; i < 40 && !status.available; i++) {
        await delay(500);
        status = await browser.call("browser.update_status", {});
      }
      const button = await cdpEval(port, "chrome.html", "(() => { const b = document.getElementById('updateButton'); return { hidden: b.hidden, text: b.innerText.trim() }; })()");
      return { status, button };
    } finally {
      await browser.close();
      fs.rmSync(root, { recursive: true, force: true });
    }
  };

  try {
    const good = await run("good");
    assert.equal(good.status.available, "9.9.9", `the signed release is found: ${JSON.stringify(good.status)}`);
    assert.equal(good.status.automatic, false);
    assert.equal(good.button.hidden, false, "the toolbar offers the update");
    assert.match(good.button.text, /Update to 9\.9\.9/);

    const forged = await run("forged");
    assert.equal(forged.status.available, undefined, "a manifest signed by another key is ignored");
    assert.equal(forged.button.hidden, true, "and nothing is offered");
    console.log("autoupdate_e2e ok");
  } finally {
    site.close();
  }
})().catch((error) => {
  console.error(error);
  process.exit(1);
});
