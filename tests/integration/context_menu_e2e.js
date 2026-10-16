// Right-click opens AgentGo's own menu (not the engine's), and its
// accessibility check and font identifier answer for the page.
const assert = require("node:assert/strict");
const http = require("node:http");
const { launch, cdpEval, delay, reservePort, refOf } = require("./harness");

const page = `<!doctype html><title>ctx</title><body style="font-family:Georgia,serif">
<a href="https://example.com/x" id="l">Example link</a>
<p>Some text</p><img src="data:image/gif;base64,R0lGODlhAQABAAAAACw=" width="40" height="40"><button></button>`;

(async () => {
  const site = http.createServer((_, response) => response.writeHead(200, { "Content-Type": "text/html" }).end(page));
  await new Promise((resolve) => site.listen(0, "127.0.0.1", resolve));
  const port = await reservePort();
  const browser = launch({ args: ["--url", "about:blank", "--mode", "full"], debugPort: port });
  const tools = (expression) => cdpEval(port, "tools.html", expression);
  let failed = false;
  try {
    await browser.call("navigate.url", { url: `http://127.0.0.1:${site.address().port}/`, observe: "none" });
    await browser.call("wait.load", {});
    const view = await browser.call("observe.page", {});
    await browser.call("act.right_click", { ref: refOf(view.page, "Example link") });
    await delay(1200);
    const menu = await tools("({ body: document.body.className, items: [...document.querySelectorAll('[data-ctx]')].map(b => b.dataset.ctx) })");
    assert.match(menu.body, /ctxmenu/, `the themed menu is open: ${JSON.stringify(menu)}`);
    for (const action of ["open_link", "copy_link", "back", "reload", "print", "inspect", "a11y", "fonts"]) {
      assert.ok(menu.items.includes(action), `menu offers ${action}: ${menu.items}`);
    }

    await tools("document.querySelector('[data-ctx=a11y]').click()");
    await delay(2500);
    const report = await tools("document.body.innerText");
    assert.match(report, /alt|name/i, `accessibility report lists problems: ${report}`);

    console.log("context_menu_e2e ok");
  } catch (error) {
    console.error(error);
    failed = true;
  } finally {
    site.close();
    await browser.close();
    process.exitCode = failed ? 1 : 0;
  }
})();
