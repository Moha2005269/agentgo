// @live
// Real ad and tracker hosts are cancelled from a real https page, ordinary
// hosts are not, ad-heavy news sites still load, and switching off restores it all.
const assert = require("node:assert/strict");
const { launch, operator, delay, reservePort } = require("./harness");

(async () => {
  const debugPort = await reservePort();
  const browser = launch({ args: ["--url", "about:blank", "--mode", "full", "--allow-evaluate"], debugPort });
  let failed = false;
  try {
    for (let i = 0; i < 100 && !(await browser.call("settings.get")).adBlockReady; i += 1) await delay(100);
    await browser.call("navigate.url", { url: "https://example.com/", observe: "none" });
    let n = 0;
    const script = async (src) => {
      const key = `s${n += 1}`;
      await browser.call("runtime.evaluate", { expression: `window.__r=window.__r||{};(()=>{const s=document.createElement('script');s.onload=()=>{window.__r.${key}=true};s.onerror=()=>{window.__r.${key}=false};s.src=${JSON.stringify(src)};document.head.append(s);return true})()` });
      for (let i = 0; i < 100; i += 1) {
        const r = JSON.parse((await browser.call("runtime.evaluate", { expression: "JSON.stringify(window.__r)" })).value);
        if (key in r) return r[key];
        await delay(100);
      }
      throw new Error(`no result for ${src}`);
    };
    const ads = [
      "https://pagead2.googlesyndication.com/pagead/js/adsbygoogle.js",
      "https://www.google-analytics.com/analytics.js",
      "https://securepubads.g.doubleclick.net/tag/js/gpt.js",
    ];
    const fine = "https://cdn.jsdelivr.net/npm/jquery@3.7.1/dist/jquery.min.js";
    const out = {};
    assert.equal(await script(fine), true, "an ordinary CDN script loads");
    for (const ad of ads) out[ad] = await script(ad);
    console.log(JSON.stringify(out));
    for (const ad of ads) assert.equal(out[ad], false, `blocked: ${ad}`);
    const blocked = (await browser.call("settings.get")).adBlockBlocked;
    assert(blocked >= ads.length);

    await operator(debugPort, "settings.set", { settings: { adBlock: false } });
    for (const ad of ads) assert.equal(await script(`${ad}?off=1`), true, `loads when off: ${ad}`);
    await operator(debugPort, "settings.set", { settings: { adBlock: true } });

    for (const url of ["https://www.cnn.com/", "https://www.bbc.com/news", "https://en.wikipedia.org/wiki/Main_Page"]) {
      await browser.call("navigate.url", { url, observe: "none" });
      await delay(4000);
      const title = (await browser.call("runtime.evaluate", { expression: "document.title + ' | ' + document.body.innerText.length" })).value;
      console.log(url, "->", title);
      assert(Number(title.split("|").pop()) > 500, `${url} rendered content`);
    }
    console.log(JSON.stringify({ ok: true, blocked: (await browser.call("settings.get")).adBlockBlocked }));
  } catch (error) {
    console.error(error);
    failed = true;
  } finally {
    await browser.close();
    process.exitCode = failed ? 1 : 0;
  }
})();
