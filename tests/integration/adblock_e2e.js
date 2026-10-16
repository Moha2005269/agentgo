// The ad blocker cancels ad and tracker subresources, never the page itself,
// can be switched off by the person (not by an agent), and counts what it blocked.
const assert = require("node:assert/strict");
const { launch, serve, operator, cdpEval, rejects, delay, reservePort } = require("./harness");

(async () => {
  const site = await serve();
  const debugPort = await reservePort();
  const browser = launch({ args: ["--url", "about:blank", "--mode", "full", "--allow-evaluate"], debugPort });
  let failed = false;
  try {
    for (let i = 0; i < 100 && !(await browser.call("settings.get")).adBlockReady; i += 1) await delay(100);
    const initial = await browser.call("settings.get");
    assert.equal(initial.adBlock, true, "ad blocking is on by default");
    assert.equal(initial.adBlockReady, true, "the engine finished loading");

    await browser.call("navigate.url", { url: site.url("adblock.html"), observe: "none" });
    const base = site.url("");
    const adUrl = `${base}ads/acctid=1/adpixel.gif`;
    const okUrl = `${base}static/okpixel.gif`;
    let seq = 0;
    const load = async (url) => {
      const key = `k${seq += 1}`;
      await browser.call("runtime.evaluate", { expression: `probe(${JSON.stringify(key)}, ${JSON.stringify(`${url}?n=${key}`)})` });
      for (let i = 0; i < 50; i += 1) {
        const r = JSON.parse((await browser.call("runtime.evaluate", { expression: "JSON.stringify(window.__r)" })).value);
        if (key in r) return r[key];
        await delay(100);
      }
      throw new Error(`no result for ${url}`);
    };

    assert.equal(await load(okUrl), true, "ordinary images load");
    assert.equal(await load(adUrl), false, "an ad URL is cancelled");
    const blocked = (await browser.call("settings.get")).adBlockBlocked;
    assert(blocked >= 1, "the counter went up");

    await rejects(browser.call("settings.set", { settings: { adBlock: false } }), "policy.denied");
    assert.equal((await browser.call("settings.get")).adBlock, true, "an agent cannot switch it off");

    await operator(debugPort, "settings.set", { settings: { adBlock: false } });
    assert.equal((await browser.call("settings.get")).adBlock, false);
    assert.equal(await load(adUrl), true, "with blocking off the ad URL loads");

    await operator(debugPort, "settings.set", { settings: { adBlock: true } });
    assert.equal(await load(adUrl), false, "switching back on blocks again");

    // Elements the lists name are hidden; the rest of the page is untouched.
    // EasyList switches generic hiding off for 127.0.0.1, so use a .localhost name.
    const cosmeticUrl = site.url("cosmetic.html").replace("127.0.0.1", "ads.localhost");
    const shown = (id) => browser.call("runtime.evaluate", { expression: `getComputedStyle(document.getElementById(${JSON.stringify(id)})).display !== 'none'` }).then((r) => r.value);
    const settle = async (id, want) => {
      for (let i = 0; i < 40; i += 1) {
        if ((await shown(id)) === want) return true;
        await delay(100);
      }
      return false;
    };
    await browser.call("navigate.url", { url: cosmeticUrl, observe: "none" });
    const hidden = await settle("banner", false);
    const why = await browser.call("runtime.evaluate", { expression: "JSON.stringify([location.href, window.__agentgoCosmetic, document.adoptedStyleSheets.length, document.styleSheets.length])" });
    assert.equal(hidden, true, `an ad banner is hidden ${why.value}`);
    assert.equal(await shown("plain"), true, "ordinary content stays");
    await operator(debugPort, "settings.set", { settings: { adBlock: false } });
    await browser.call("navigate.url", { url: cosmeticUrl, observe: "none" });
    assert.equal(await settle("banner", true), true, "with blocking off the banner shows");
    await operator(debugPort, "settings.set", { settings: { adBlock: true } });

    const ui = await cdpEval(debugPort, "chrome.html", "true");
    assert.equal(ui, true);
    console.log(JSON.stringify({ ok: true, blocked: (await browser.call("settings.get")).adBlockBlocked }));
  } catch (error) {
    console.error(`${error.message} | ${(error.stack || "").split("\n").find((line) => line.includes("adblock_e2e")) || ""}`);
    failed = true;
  } finally {
    await browser.close();
    site.close();
    process.exitCode = failed ? 1 : 0;
  }
})();

