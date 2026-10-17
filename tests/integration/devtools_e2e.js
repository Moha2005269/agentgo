// Developer tools: right-click Inspect opens them docked; Inspect again
// inspects another element in the same tools; the close button in their
// corner hides them; they come back; closing the inspected tab with its
// tools open keeps the browser working. Each step checks that the toolbar
// (chrome.html) still answers: the engine used to tear the toolbar down when
// its last developer tools window closed (D88).
const assert = require("node:assert/strict");
const http = require("node:http");
const { launch, cdpEval, delay, reservePort, refOf } = require("./harness");

const page = `<!doctype html><title>dt</title><body><h1>Hello</h1><p>Some text</p><a href="https://example.com/">Example link</a>`;

(async () => {
  const site = http.createServer((_, r) => r.writeHead(200, { "Content-Type": "text/html" }).end(page));
  await new Promise((r) => site.listen(0, "127.0.0.1", r));
  const url = `http://127.0.0.1:${site.address().port}/`;
  const port = await reservePort();
  const browser = launch({ args: ["--url", "about:blank", "--mode", "full"], debugPort: port });
  const withTimeout = (p, ms) => Promise.race([p, delay(ms).then(() => { throw new Error("timed out"); })]);
  const toolbarAnswers = async (step) => {
    const width = await withTimeout(cdpEval(port, "chrome.html", "innerWidth"), 5000).catch((e) => e.message);
    assert.equal(typeof width, "number", `${step}: the toolbar still answers (${width})`);
  };
  const devtoolsTargets = async () => (await (await fetch(`http://127.0.0.1:${port}/json/list`)).json()).filter((t) => t.url.startsWith("devtools://")).length;
  const inspect = async () => {
    const view = await browser.call("observe.page", {});
    await browser.call("act.right_click", { ref: refOf(view.page, "Example link") });
    await delay(900);
    await cdpEval(port, "tools.html", "document.querySelector('[data-ctx=inspect]').click()");
    await delay(3000);
  };
  const closeButton = (action) => withTimeout(cdpEval(port, "devtools://devtools/bundled/devtools_app.html", `(() => { const b = document.getElementById('agentgo-close'); if (!b) return 'missing'; ${action} return 'ok'; })()`), 5000);
  try {
    await browser.call("navigate.url", { url, observe: "none" });
    await browser.call("wait.load", {});

    await inspect();
    assert.ok(await devtoolsTargets() >= 1, "developer tools opened");
    assert.equal(await closeButton(""), "ok", "the docked tools show a close button");
    await toolbarAnswers("opened");

    await inspect();
    await toolbarAnswers("inspected again while open");

    assert.equal(await closeButton("b.click();"), "ok");
    await delay(1500);
    await toolbarAnswers("closed with the X");

    await inspect();
    await toolbarAnswers("opened again");

    const first = (await browser.call("tabs.list", {})).tabs[0].tabId;
    await browser.call("tabs.create", { url });
    await delay(1500);
    await browser.call("tabs.close", { tabId: first });
    await delay(2500);
    await toolbarAnswers("closed the inspected tab");
    const after = await browser.call("observe.page", {});
    assert.match(after.page, /Example link/, "the remaining tab still works");
    console.log("devtools_e2e ok");
  } finally {
    site.close();
    await browser.close();
  }
})().catch((error) => {
  console.error(error);
  process.exit(1);
});
