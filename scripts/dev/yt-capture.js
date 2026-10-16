// Records what YouTube sends for a watch page, with the ad blocker on or off:
// the /youtubei/v1/player response (which ad keys it carries), any googlevideo
// URLs with ad parameters, and whether #movie_player ever shows an ad.
//   AGENTGO_EXE=... node scripts/dev/run-integration.js  (needs a private desktop)
//   target\agentgo-tool.exe quiet-run -- node scripts/dev/yt-capture.js [on|off] [videoId] [seconds]
const fs = require("node:fs");
const path = require("node:path");
const { launch, operator, reservePort, delay } = require("../../tests/integration/harness");

const mode = process.argv[2] || "off";
const video = process.argv[3] || "dQw4w9WgXcQ";
const seconds = Number(process.argv[4] || 45);
const AD_KEYS = ["adPlacements", "playerAds", "adSlots", "adBreakHeartbeatParams", "auxiliaryUi"];

(async () => {
  const port = await reservePort();
  const browser = launch({ args: ["--url", "about:blank"], debugPort: port });
  const report = { mode, video, player: [], adVideoUrls: [], states: [], blockedYoutube: [] };
  try {
    await operator(port, "settings.set", { settings: { adBlock: mode === "on", vpn: process.env.YT_VPN !== "0" } });
    for (let i = 0; i < 60; i += 1) { await delay(1000); const s = await browser.call("settings.get"); if (s.vpnState === "ready") break; }
    await delay(1500);
    await browser.call("navigate.url", { url: `https://www.youtube.com/watch?v=${video}`, observe: "none" });
    let target;
    for (let i = 0; i < 100 && !target; i += 1) {
      const list = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
      target = list.find((t) => t.url.includes("youtube.com"));
      if (!target) await delay(200);
    }
    const socket = new WebSocket(target.webSocketDebuggerUrl);
    await new Promise((resolve) => socket.addEventListener("open", resolve, { once: true }));
    let next = 1;
    const pending = new Map();
    const urls = new Map();
    const send = (method, params = {}) => new Promise((resolve) => {
      const id = next++;
      pending.set(id, resolve);
      socket.send(JSON.stringify({ id, method, params }));
    });
    socket.addEventListener("message", async (event) => {
      const m = JSON.parse(event.data);
      if (m.id && pending.has(m.id)) { pending.get(m.id)(m.result); pending.delete(m.id); return; }
      if (m.method === "Network.requestWillBeSent") urls.set(m.params.requestId, m.params.request.url);
      if (m.method === "Network.loadingFailed" && /youtube|googlevideo|doubleclick/.test(urls.get(m.params.requestId) || "")) {
        report.blockedYoutube.push((urls.get(m.params.requestId) || "").slice(0, 160));
      }
      if (m.method === "Network.responseReceived") {
        const url = m.params.response.url;
        if (/googlevideo\.com\/videoplayback/.test(url) && /[?&](adformat|oad|ctier|ad_type|ctier)=/.test(url)) {
          report.adVideoUrls.push(url.slice(0, 220));
        }
      }
      if (m.method === "Network.loadingFinished") {
        const url = urls.get(m.params.requestId) || "";
        if (/youtubei\/v1\/player/.test(url)) {
          const body = await send("Network.getResponseBody", { requestId: m.params.requestId });
          try {
            const json = JSON.parse(body.body);
            report.player.push({ url: url.slice(0, 100), keys: AD_KEYS.filter((k) => k in json), status: json.playabilityStatus?.status });
          } catch (_) { report.player.push({ url: url.slice(0, 100), unparsed: true }); }
        }
      }
    });
    await send("Network.enable");
    for (let t = 0; t < seconds; t += 1) {
      await delay(1000);
      try {
        const r = await send("Runtime.evaluate", { returnByValue: true, expression: `(()=>{const p=document.querySelector('#movie_player');const v=document.querySelector('video');return JSON.stringify({ad:!!(p&&p.classList.contains('ad-showing')),t:v?+v.currentTime.toFixed(1):null,paused:v?v.paused:null,muted:v?v.muted:null,warn:!!document.querySelector('ytd-enforcement-message-view-model,tp-yt-paper-dialog #dismiss-button')})})()` });
        report.states.push(JSON.parse(r.result.value));
      } catch (_) {}
    }
    const init = await send("Runtime.evaluate", { returnByValue: true, expression: "JSON.stringify(Object.keys(window.ytInitialPlayerResponse||{}).filter(k=>/ad/i.test(k)))" });
    report.initialPlayerAdKeys = init.result && init.result.value;
    socket.close();
    report.adSeconds = report.states.filter((s) => s.ad).length;
    report.maxTime = Math.max(0, ...report.states.map((s) => s.t || 0));
  } finally {
    await browser.close();
  }
  const out = path.join(__dirname, "..", "..", "target", `yt-capture-${mode}.json`);
  fs.writeFileSync(out, JSON.stringify(report, null, 1));
  console.log(JSON.stringify({ ...report, states: undefined, stateSample: report.states.filter((_, i) => i % 5 === 0) }, null, 1));
})().catch((error) => { console.error(error); process.exit(1); });
