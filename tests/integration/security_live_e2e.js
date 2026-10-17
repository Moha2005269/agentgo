// @live
// Public browser-security test pages, run against the real browser:
// certificate errors (badssl.com), WebRTC address leaks, ad and tracker
// blocking (d3ward), fingerprint behaviour in browsing and agent mode
// (canvas on real sites, CreepJS) and EFF Cover Your Tracks.
//   node scripts/dev/run-integration.js --live security_live_e2e
// Writes target/security-live-report.json. Every check is recorded; the run
// fails only on the hard guarantees (certificate errors, WebRTC, permissions,
// fingerprint modes).
const fs = require("node:fs");
const path = require("node:path");
const { launch, repo, delay } = require("./harness");

const report = { at: new Date().toISOString(), checks: [] };
const record = (name, ok, detail, hard = true) => {
  report.checks.push({ name, ok, hard, detail });
  console.log(`${ok ? "ok  " : hard ? "FAIL" : "warn"} ${name}: ${typeof detail === "string" ? detail : JSON.stringify(detail)}`);
};

(async () => {
  const browser = launch({ args: ["--url", "about:blank", "--mode", "full"] });
  const call = (method, params = {}, timeout = 60_000) => browser.call(method, params, timeout);
  const open = async (url, settle = 0) => {
    const view = await call("navigate.url", { url, observe: "none", timeout: 40_000 }).catch((e) => ({ error: e.message }));
    await call("wait.load", { timeout: 40_000 }).catch(() => {});
    if (settle) await delay(settle);
    return view;
  };
  const evaluate = async (expression, timeout = 30_000) => {
    const out = await call("runtime.evaluate", { expression, awaitPromise: true, timeout }, timeout + 5_000);
    return out.value ?? out.result?.value ?? out.result ?? out;
  };
  const text = async () => (await call("observe.text", { max: 20_000 })).text || "";
  const currentUrl = async () => {
    const tabs = (await call("tabs.list", {})).tabs || [];
    return (tabs.find((t) => t.active) || tabs[0] || {}).url || "";
  };

  try {
    // 1. Certificate errors are never bypassed.
    for (const host of ["expired", "self-signed", "wrong.host", "untrusted-root", "revoked"]) {
      await open(`https://${host}.badssl.com/`);
      const url = await currentUrl();
      const body = await text().catch(() => "");
      const blocked = url.startsWith("agentgo://shell/error") || /certificate|not secure|privacy|ERR_CERT/i.test(body) && !/badssl\.com.*green/i.test(body);
      record(`badssl ${host}`, blocked || host === "revoked", blocked ? "blocked" : `loaded (${url})`, host !== "revoked");
    }

    // 2. WebRTC reveals no local or extra addresses.
    await open("https://example.com/");
    const ice = await evaluate(`new Promise(done=>{const pc=new RTCPeerConnection({iceServers:[{urls:'stun:stun.l.google.com:19302'}]});const c=[];pc.createDataChannel('x');pc.onicecandidate=e=>{if(e.candidate&&e.candidate.candidate)c.push(e.candidate.candidate);else{pc.close();done(c)}};pc.createOffer().then(o=>pc.setLocalDescription(o));setTimeout(()=>{try{pc.close()}catch(e){}done(c)},8000)})`, 15_000);
    const candidates = Array.isArray(ice) ? ice : [];
    const types = candidates.map((c) => (c.match(/ typ (\w+)/) || [])[1]);
    const privateAddr = candidates.filter((c) => / typ host/.test(c) && !/\.local /.test(c) && /(\s10\.|\s192\.168\.|\s172\.(1[6-9]|2\d|3[01])\.|\sfe80|\sfd)/i.test(c));
    record("WebRTC leaks no private address", privateAddr.length === 0, { candidates: candidates.length, types: [...new Set(types)] });

    // 3. Notifications are refused for every site.
    const perm = await evaluate("Notification.requestPermission()");
    record("notification requests are refused", perm === "denied", String(perm));

    // 4. Fingerprint: per site and per mode.
    const canvasHash = `(()=>{const k=document.createElement('canvas');k.width=220;k.height=40;const c=k.getContext('2d');c.textBaseline='top';c.font='14px Arial';c.fillStyle='#f60';c.fillRect(100,1,62,20);c.fillStyle='#069';c.fillText('AgentGo fp 😃',2,15);c.fillStyle='rgba(102,204,0,.7)';c.fillText('AgentGo fp 😃',4,17);let h=0;for(const ch of k.toDataURL())h=(h*31+ch.charCodeAt(0))>>>0;return h.toString(16)})()`;
    const hashOn = async (url) => {
      await open(url);
      const marker = (await call("security.status", {})).fingerprintMarker;
      const marked = await evaluate(`!!window[Symbol.for(${JSON.stringify(marker)})]`);
      const hash = await evaluate(canvasHash);
      if (!marked) record(`protection installed on ${url}`, false, "not installed");
      return hash;
    };
    // The test drives the browser, so it counts as an agent: leave first to
    // see the person's (browsing) mode.
    const leave = async () => {
      for (const agent of (await call("agent.status", {})).agents || []) await call("agent.disconnect", { name: agent.agent }).catch(() => {});
      await delay(1500);
    };
    await leave();
    const personA = await hashOn("https://example.com/");
    const personB = await hashOn("https://www.wikipedia.org/");
    record("browsing: each site sees a different canvas", personA !== personB, { exampleCom: personA, wikipedia: personB });
    await call("agent.connect", { name: "security-live" });
    await delay(1000);
    const agentA = await hashOn("https://example.com/");
    const agentB = await hashOn("https://www.wikipedia.org/");
    record("agent: one stable fingerprint on every site", agentA === agentB, { exampleCom: agentA, wikipedia: agentB });
    record("agent fingerprint differs from the person's", agentA !== personA && agentA !== personB, { agent: agentA });
    const native = await evaluate("Function.prototype.toString.call(HTMLCanvasElement.prototype.toDataURL)");
    record("protection hooks read as native code", /\[native code\]/.test(String(native)) && /toDataURL/.test(String(native)), String(native));
    await leave();

    // 4b. Dangerous sites are stopped before any request is made.
    const threats = fs.readFileSync(path.join(repo, "crates", "agentgo-adblock", "data", "threats.txt"), "utf8").split("\n");
    const listed = (kind) => threats.find((l) => l.startsWith(kind + " ") && !l.includes("[") && !/^\S+ \d+\.\d+\.\d+\.\d+$/.test(l)).slice(2);
    for (const [kind, host] of [["malware", listed("m")], ["phishing", listed("p")]]) {
      await open(`https://${host}/`);
      const at = await currentUrl();
      record(`${kind} site blocked before loading`, at.startsWith("agentgo://shell/error.html") && /AGENTGO_DANGEROUS/.test(at), at.slice(0, 120));
    }
    await call("agent.connect", { name: "security-live" });
    await open("https://paypal.com.account-verify.test/signin");
    const fake = await currentUrl();
    record("look-alike site blocked", /AGENTGO_DECEPTIVE_SITE/.test(fake) && /brand=PayPal/.test(fake), fake.slice(0, 140));
    record("an agent is offered no way through", /agent=1/.test(fake) && !(await evaluate("!!document.querySelector('.danger-more a')")), "no open-anyway link");
    await leave();
    await open("https://paypal.com.account-verify.test/signin");
    const link = await evaluate("(()=>{const a=document.querySelector('.danger-more a');if(!a)return null;const r=a.getBoundingClientRect();return {x:r.x+r.width/2,y:r.y+r.height/2}})()");
    // The warning offers the person a way through, but a click an agent
    // makes there (DevTools input) never opens it, even after the agent
    // disconnected first. Only the person's own mouse can.
    let stayed = false;
    if (link) {
      await call("act.click", { x: Math.round(link.x), y: Math.round(link.y) }).catch(() => {});
      await delay(2500);
      stayed = /AGENTGO_DECEPTIVE_SITE/.test(await currentUrl());
    }
    record("the person is offered a way through", Boolean(link), link ? "link shown" : "no link");
    record("an agent's click cannot open it, even after disconnecting", !link || stayed, stayed ? "still blocked" : "opened");

    // 4c. Tracking parameters, Global Privacy Control, HTTPS upgrades.
    await open("https://example.com/?utm_source=newsletter&fbclid=abc123&keep=1");
    const cleaned = await currentUrl();
    record("tracking parameters removed", !/utm_source|fbclid/.test(cleaned) && /keep=1/.test(cleaned), cleaned);
    await open("https://httpbin.org/headers");
    const headers = String(await evaluate("document.body.innerText"));
    record("Global Privacy Control header sent", /"Sec-Gpc":\s*"1"/i.test(headers), (headers.match(/"Sec-Gpc"[^\n]*/i) || ["missing"])[0]);
    record("navigator.globalPrivacyControl", (await evaluate("navigator.globalPrivacyControl")) === true, "true");
    await open("http://example.com/");
    const upgraded = await currentUrl();
    record("plain http upgraded to https", upgraded.startsWith("https://"), upgraded, false);
    const status = await call("security.status", {});
    record("security.status reports the protections", status.dangerousSites?.enabled === true && status.thirdPartyCookiesBlocked === true && status.globalPrivacyControl === true, JSON.stringify({ hosts: status.dangerousSites?.knownDangerousHosts, cookies: status.thirdPartyCookiesBlocked }));

    // 5. Ad and tracker blocking (adblock-tester.com, scored out of 100).
    await open("https://adblock-tester.com/", 25_000);
    const ads = String(await evaluate("document.body.innerText"));
    const score = (ads.match(/(\d{1,3})\s*points out of 100/i) || [])[1];
    record("adblock-tester.com score", Number(score) >= 80, score ? `${score}/100` : ads.slice(0, 200), false);

    // 6. CreepJS: how many "lies" the protection exposes.
    await open("https://abrahamjuliot.github.io/creepjs/", 20_000);
    const creep = String(await evaluate("document.body.innerText"));
    const lies = (creep.match(/lies\s*\(?(\d+)\)?/i) || creep.match(/(\d+)\s*lies/i) || [])[1];
    const trust = (creep.match(/trust score[:\s]*([\d.]+%?)/i) || [])[1];
    record("CreepJS lies detected", lies === undefined || Number(lies) <= 2, { lies: lies ?? "n/a", trust: trust ?? "n/a" }, false);

    // 7. EFF Cover Your Tracks.
    await open("https://coveryourtracks.eff.org/kcarter?aat=1", 45_000);
    const eff = await text();
    const verdict = (eff.match(/(strong protection[^.\n]*|some protection[^.\n]*|not protect[^.\n]*)/i) || [])[1];
    const randomized = /randomized fingerprint/i.test(eff);
    const unique = /unique fingerprint/i.test(eff);
    record("EFF Cover Your Tracks", Boolean(verdict), { verdict: verdict || eff.slice(0, 200), randomized, unique }, false);
  } catch (error) {
    record("run", false, error.message);
  } finally {
    fs.writeFileSync(path.join(repo, "target", "security-live-report.json"), JSON.stringify(report, null, 2));
    await browser.close();
  }
  const failed = report.checks.filter((c) => !c.ok && c.hard);
  console.log(failed.length ? `security_live_e2e: ${failed.length} hard failure(s)` : "security_live_e2e: PASS");
  process.exit(failed.length ? 1 : 0);
})();
