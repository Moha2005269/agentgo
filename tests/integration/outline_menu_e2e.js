// A script-driven menu (rows are divs with the pointer cursor inside a pointer
// container, no roles) appears in the page outline with one target per row.
const assert = require("node:assert/strict");
const http = require("node:http");
const { launch } = require("./harness");

const page = `<!doctype html><title>menu</title><style>.menu,.menu div{cursor:pointer}</style>
<div class="menu" id="m"><div>Open file</div><div>Save as</div><div>Export PDF</div></div>`;

(async () => {
  const site = http.createServer((_, response) => response.writeHead(200, { "Content-Type": "text/html" }).end(page));
  await new Promise((resolve) => site.listen(0, "127.0.0.1", resolve));
  const browser = launch({ args: ["--url", "about:blank"] });
  try {
    await browser.call("navigate.url", { url: `http://127.0.0.1:${site.address().port}/`, observe: "none" });
    await browser.call("wait.load", {});
    const view = await browser.call("observe.page", {});
    const text = view.page || view.text || JSON.stringify(view);
    for (const row of ["Open file", "Save as", "Export PDF"]) {
      assert.ok(new RegExp(`"${row}"`).test(text), `the outline lists the menu row ${row}:\n${text}`);
    }
    console.log("outline_menu_e2e ok");
  } finally {
    site.close();
    await browser.close();
  }
})().catch((error) => {
  console.error(error);
  process.exit(1);
});
