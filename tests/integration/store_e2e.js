// @live
// Chrome Web Store: on an extension's store page the toolbar offers "Add to
// AgentGo"; installing fetches the package from Google's update service and
// its content scripts run on matching pages. Needs the internet.
//   node scripts/dev/run-integration.js --live store_e2e
const assert = require("node:assert/strict");
const { launch, reservePort, operator, cdpEval, delay } = require("./harness");

(async () => {
  const port = await reservePort();
  const browser = launch({ args: ["--url", "about:blank", "--mode", "full", "--allow-evaluate"], debugPort: port });
  let failed = false;
  try {
    const page = "https://chromewebstore.google.com/detail/dark-reader/eimadpbcbfnmbkopoojfekhnkhdbieeh";
    await operator(port, "tabs.navigate", { url: page });
    let shown = false;
    for (let i = 0; i < 50 && !shown; i += 1) {
      await delay(200);
      shown = await cdpEval(port, "chrome.html", "!document.querySelector('#addExtension').hidden").catch(() => false);
    }
    assert(shown, "the Add to AgentGo button did not appear on the store page");
    const added = await operator(port, "extension.install", { store: page });
    assert.equal(added.name, "Dark Reader");
    assert(added.contentScripts.length >= 1);
    await browser.call("navigate.url", { url: "https://example.com/" });
    let styled = 0;
    for (let i = 0; i < 20 && !styled; i += 1) {
      await delay(250);
      styled = (await browser.call("runtime.evaluate", { expression: "document.querySelectorAll('style.darkreader, .darkreader').length" })).value;
    }
    assert(styled > 0, "the extension's content script did not run");
    assert.equal((await operator(port, "extension.uninstall", { id: added.id })).uninstalled, true);
    console.log(JSON.stringify({ ok: true }));
  } catch (error) {
    console.error(error);
    failed = true;
  } finally {
    const code = await browser.close();
    process.exitCode = failed || code !== 0 ? 1 : 0;
  }
})();
