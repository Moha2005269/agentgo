// YouTube ad removal, against a local stand-in for YouTube. The updatable rules
// and module (data files in the browser's data folder) point at 127.0.0.1, which
// proves two things at once: the network layer strips ad data from the player
// response and the page's inline copy before the page reads them, and the
// isolated-world module mutes, speeds up and skips an in-stream ad and removes
// the "ad blockers are not allowed" wall, while the page's own objects stay
// untouched.
const assert = require("node:assert/strict");
const fs = require("node:fs");
const http = require("node:http");
const os = require("node:os");
const path = require("node:path");
const { launch, operator, reservePort, delay, cdpEval } = require("./harness");

const player = { playabilityStatus: { status: "OK" }, videoDetails: { title: "t", pad: "x".repeat(3000000) }, adPlacements: [{ x: 1 }], playerAds: [{ y: 2 }], adSlots: [{ z: 3 }] };
const watch = `<!doctype html><!-- ${"y".repeat(2000000)} --><title>watch</title><div id="movie_player" class="html5-video-player"><video class="html5-main-video"></video><button class="ytp-skip-ad-button" onclick="window.__skipClicked=(window.__skipClicked||0)+1">Skip</button></div>
<div id="wall"><ytd-enforcement-message-view-model>Ad blockers violate the Terms</ytd-enforcement-message-view-model></div>
<script>var ytInitialPlayerResponse = ${JSON.stringify(player)};
window.__inlineKeys = Object.keys(ytInitialPlayerResponse).join(",");
fetch('/youtubei/v1/player?key=1',{method:'POST'}).then(r=>r.json()).then(j=>{window.__fetchKeys=Object.keys(j).join(",")});</script>`;

(async () => {
  const site = http.createServer((request, response) => {
    if (request.url.startsWith("/youtubei/v1/player")) response.writeHead(200, { "Content-Type": "application/json" }).end(JSON.stringify(player));
    else response.writeHead(200, { "Content-Type": "text/html" }).end(watch);
  });
  await new Promise((resolve) => site.listen(0, "127.0.0.1", resolve));
  const base = `http://127.0.0.1:${site.address().port}`;
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "agentgo-yt-"));
  const port = await reservePort();
  const browser = launch({ args: ["--url", "about:blank"], debugPort: port, dataRoot: root });
  try {
    // The data folder is where state.db lives.
    let dataDir;
    for (let i = 0; i < 100 && !dataDir; i += 1) {
      const find = (dir, depth = 0) => {
        for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
          if (entry.isFile() && entry.name === "state.db") return dir;
          if (entry.isDirectory() && depth < 3) { const hit = find(path.join(dir, entry.name), depth + 1); if (hit) return hit; }
        }
        return null;
      };
      dataDir = find(root);
      if (!dataDir) await delay(200);
    }
    assert.ok(dataDir, "found the data folder");
    const adblockDir = path.join(dataDir, "adblock");
    fs.mkdirSync(adblockDir, { recursive: true });
    const rules = JSON.parse(fs.readFileSync(path.join(__dirname, "..", "..", "crates", "agentgo-adblock", "data", "youtube-rules.json"), "utf8"));
    rules.hosts = ["127.0.0.1"];
    fs.writeFileSync(path.join(adblockDir, "youtube-rules.json"), JSON.stringify(rules));
    const module = fs.readFileSync(path.join(__dirname, "..", "..", "crates", "agentgo-adblock", "data", "youtube-module.js"), "utf8")
      .replace("/(^|\\.)youtube(-nocookie)?\\.com$/", "/^127\\.0\\.0\\.1$/");
    assert.ok(module.includes("127\\.0\\.0\\.1"), "module host gate replaced for the test");
    fs.writeFileSync(path.join(adblockDir, "youtube-module.js"), module);
    // Turning the blocker off and on makes the browser re-read the data files.
    await operator(port, "settings.set", { settings: { adBlock: false } });
    await operator(port, "settings.set", { settings: { adBlock: true } });
    await delay(1500);

    await browser.call("navigate.url", { url: `${base}/watch`, observe: "none" });
    await browser.call("wait.load", {});
    await delay(1500);
    const keys = await cdpEval(port, "127.0.0.1", "[window.__inlineKeys, window.__fetchKeys].join('|')");
    const [inline, fetched] = keys.split("|");
    assert.equal(inline, "playabilityStatus,videoDetails", `inline player response was cleaned: ${inline}`);
    assert.equal(fetched, "playabilityStatus,videoDetails", `fetched player response was cleaned: ${fetched}`);

    // The wall is gone.
    assert.equal(await cdpEval(port, "127.0.0.1", "document.querySelectorAll('ytd-enforcement-message-view-model').length"), 0, "enforcement wall removed");

    // An ad in the stream: muted, sped up, skip pressed; then restored.
    await cdpEval(port, "127.0.0.1", "document.querySelector('#movie_player').classList.add('ad-showing'); 1");
    await delay(800);
    const during = JSON.parse(await cdpEval(port, "127.0.0.1", "JSON.stringify({m:document.querySelector('video').muted,r:document.querySelector('video').playbackRate,s:window.__skipClicked||0})"));
    assert.equal(during.m, true, "muted during the ad");
    assert.equal(during.r, 16, "sped up during the ad");
    assert.ok(during.s >= 1, "skip button pressed");
    await cdpEval(port, "127.0.0.1", "document.querySelector('#movie_player').classList.remove('ad-showing'); 1");
    await delay(800);
    const after = JSON.parse(await cdpEval(port, "127.0.0.1", "JSON.stringify({m:document.querySelector('video').muted,r:document.querySelector('video').playbackRate})"));
    assert.deepEqual(after, { m: false, r: 1 }, "volume and speed restored after the ad");

    // The page cannot see the module.
    assert.equal(await cdpEval(port, "127.0.0.1", "typeof window.__agentgoYoutube"), "undefined", "module invisible to the page");
    console.log("youtube_e2e ok");
  } finally {
    site.close();
    await browser.close();
    fs.rmSync(root, { recursive: true, force: true });
  }
})().catch((error) => {
  console.error(error);
  process.exit(1);
});
