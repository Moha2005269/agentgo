// @live
// Live-site check and observation-size measurement. Loads real public sites,
// observes each with the compact outline, performs one harmless trusted
// action, and records how large each representation of the page is.
// Needs network access, so the default integration run skips it; run it with
//   node scripts/dev/run-integration.js --live
// Writes docs/benchmarks/observation-size.md when run with --write.
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { launch, repo } = require("./harness");

const sites = [
  "https://example.com/",
  "https://www.google.com/",
  "https://github.com/",
  "https://developer.mozilla.org/en-US/",
  "https://www.w3.org/",
  "https://en.wikipedia.org/wiki/Web_browser",
  "https://www.rust-lang.org/",
  "https://www.chromium.org/Home/",
  "https://news.ycombinator.com/",
  "https://stackoverflow.com/questions",
];
const tokens = (chars) => Math.round(chars / 4); // rough, stated as an estimate

(async () => {
  // Full mode + evaluate grant only so the test can measure the raw DOM.
  const browser = launch({ args: ["--url", "about:blank", "--mode", "full", "--allow-evaluate"] });
  const rows = [];
  let failed = false;
  try {
    for (const url of sites) {
      const started = Date.now();
      const opened = await browser.call("navigate.url", { url, timeout: 45_000, max: 400 }, 70_000);
      const ms = Date.now() - started;
      assert(opened.url.startsWith("http"), `${url} did not load: ${JSON.stringify(opened).slice(0, 200)}`);
      const sizes = await browser.call("runtime.evaluate", {
        // A verification challenge can leave the document without a body.
        expression: "({html: document.documentElement.outerHTML.length, text: (document.body?.innerText || '').length, nodes: document.querySelectorAll('*').length})",
      });
      const outline = opened.page || "";
      const refs = (outline.match(/^\[\d+\]/gm) || []).length;
      // A harmless real action must still work on the live page.
      const scrolled = await browser.call("act.scroll", { pages: 1 });
      assert.equal(typeof scrolled.scroll?.y, "number", `${url} did not accept a scroll`);
      const text = await browser.call("observe.text", { max: 2000 });
      rows.push({
        url, title: opened.title, ms, loaded: opened.loaded,
        html: sizes.value.html, nodes: sizes.value.nodes, innerText: sizes.value.text,
        outline: outline.length, outlineTotal: opened.total, refs,
        challenged: Boolean(opened.signals?.captcha),
        textSample: text.text.length,
      });
      console.error(`${url} — outline ${outline.length} chars (${refs} refs) vs HTML ${sizes.value.html} chars`);
      if (opened.handoff) await browser.call("tabs.navigate", { url: "about:blank" });
    }
    assert.equal(rows.length, sites.length);
    const sum = (key) => rows.reduce((total, row) => total + row[key], 0);
    const summary = {
      sites: rows.length,
      htmlChars: sum("html"),
      outlineChars: sum("outline"),
      ratio: +(sum("html") / Math.max(1, sum("outline"))).toFixed(1),
      challenged: rows.filter((row) => row.challenged).length,
    };
    if (process.argv.includes("--write")) {
      const out = path.join(repo, "docs", "benchmarks");
      fs.mkdirSync(out, { recursive: true });
      const table = rows.map((r) => `| ${r.url} | ${r.nodes} | ${r.html} | ${r.innerText} | ${r.outline} | ${r.refs}${r.outlineTotal > 400 ? ` of ${r.outlineTotal}` : ""} | ${(r.html / Math.max(1, r.outline)).toFixed(0)}× | ${r.ms} |`).join("\n");
      fs.writeFileSync(path.join(out, "observation-size.md"), `# Observation size on live sites

Measured ${new Date().toISOString().slice(0, 10)} by \`tests/integration/site_render_smoke.js --write\` on the
development machine, AgentGo ${"0.1.0"}. One run, live network, so numbers move with
the sites; treat them as an order of magnitude, not a benchmark result.

What is compared: the characters an agent must read to know what it can do on
the page. \`HTML\` is \`document.documentElement.outerHTML\`; \`innerText\` is the
visible text; \`outline\` is the first page (up to 400 lines) of \`observe.page\`
(headings plus interactive elements with references). Tokens are roughly
characters / 4.

| Site | DOM nodes | HTML chars | innerText chars | outline chars | refs | HTML ÷ outline | load+observe ms |
|---|---:|---:|---:|---:|---:|---:|---:|
${table}

Total: HTML ${summary.htmlChars} chars (~${tokens(summary.htmlChars)} tokens) vs outline ${summary.outlineChars} chars
(~${tokens(summary.outlineChars)} tokens): **${summary.ratio}× smaller** over ${summary.sites} sites.
Human-verification challenges met during the run: ${summary.challenged}.

## What this does and does not show

- It shows the size of AgentGo's default observation against the raw page.
- It is **not** the A/B/C0/C1 experiment of the graduation roadmap: no agent
  solved tasks here, no success rate was measured, and Chrome + browser-use was
  not run. Browser-use also reduces the DOM before sending it to a model, so
  the fair token comparison is that later controlled experiment.
- After the first observation an action returns only a delta, which is where
  most multi-step savings come from; that is measured per task, not here.
`);
    }
    console.log(JSON.stringify({ ok: true, ...summary }));
  } catch (error) {
    console.error(error);
    failed = true;
  } finally {
    const code = await browser.close();
    process.exitCode = failed || code !== 0 ? 1 : 0;
  }
})();
