// Settings search ranks the right row first for exact words, synonyms and
// typos, offers the closest rows for near misses, and the VPN row is off by default.
const assert = require("node:assert/strict");
const { launch, operator, cdpEval, delay, reservePort } = require("./harness");

(async () => {
  const port = await reservePort();
  const browser = launch({ args: ["--url", "about:blank", "--mode", "full"], debugPort: port });
  let failed = false;
  try {
    await browser.call("api.health");
    const settings = await browser.call("settings.get");
    assert.equal(settings.vpn, false, "VPN is off by default");
    assert.equal(settings.vpnState, "off");
    
    

    await operator(port, "ui.open_tool", { tool: "settings" });
    await delay(1500);
    const top = (query) => cdpEval(port, "tools.html", `(async () => {
      toolFilter.value = ${JSON.stringify(query)};
      toolFilter.dispatchEvent(new Event('input'));
      await new Promise(r => setTimeout(r, 120));
      const hits = [...document.querySelectorAll('.results .hit .setting b')].map(b => b.textContent);
      return { hits, did: !!document.querySelector('.res-did'), empty: !!document.querySelector('.res-empty'), count: document.querySelector('#searchCount').textContent };
    })()`);
    const expectations = [
      ["theme", "Theme"], ["dark", "Dark mode for websites"], ["dark theme", "Theme"], ["ads", "Block ads and trackers"], ["adblock", "Block ads and trackers"],
      ["passwrod", "Offer to save sign-ins"],
      ["cookies", "Clear browsing data"], ["gpu", "Hardware acceleration"], ["languge", "Preferred language for pages"],
      ["camera", "Site permissions"], ["google", "Address bar and New Tab"],
    ];
    for (const [query, title] of expectations) {
      const result = await top(query);
      assert.equal(result.hits[0], title, `"${query}" finds ${title}, got ${JSON.stringify(result.hits)}`);
    }
    const near = await top("advertizing");
    assert(near.hits.includes("Block ads and trackers"), `typo reaches the ad blocker: ${JSON.stringify(near.hits)}`);
    const nothing = await top("qqqxzzv");
    assert.equal(nothing.empty, true, "gibberish shows the empty state");
    const cleared = await top("");
    assert.equal(cleared.hits.length, 0);
    assert.equal(await cdpEval(port, "tools.html", "document.querySelectorAll('.section').length > 5 && getComputedStyle(document.querySelector('.section')).display !== 'none'"), true, "clearing the search brings every section back");

    // The VPN is gone: switching it on does nothing and the row is a note.
    await operator(port, "settings.set", { settings: { vpn: true } });
    assert.equal((await browser.call("settings.get")).vpnState, "off");
    assert.equal(await cdpEval(port, "tools.html", "!!document.querySelector('[data-vpn]') && !document.querySelector('[data-setting=vpn]')"), true, "the VPN row has no switch");
    console.log(JSON.stringify({ ok: true }));
  } catch (error) {
    console.error(error);
    failed = true;
  } finally {
    await browser.close();
    process.exitCode = failed ? 1 : 0;
  }
})();

async function rejectsPolicy(promise) {
  const error = await promise.then(() => null, (e) => e);
  assert(error && error.code === "policy.denied", `expected policy.denied, got ${error && error.message}`);
}
