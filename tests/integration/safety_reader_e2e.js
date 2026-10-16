// Download safety and everyday page tools: a finished download is marked as
// from the internet and scanned by Microsoft Defender (when present); reader
// view opens and closes; the menu's pin and group actions reach the tab strip.
const assert = require("node:assert/strict");
const http = require("node:http");
const { launch, operator, cdpEval, reservePort, delay } = require("./harness");

(async () => {
  const site = http.createServer((request, response) => {
    if (request.url === "/file.txt") {
      response.writeHead(200, { "Content-Type": "application/octet-stream", "Content-Disposition": "attachment; filename=safe.txt" }).end("plain harmless text");
    } else {
      response.writeHead(200, { "Content-Type": "text/html" }).end("<title>Article</title><article><h1>Headline</h1><p>First paragraph of the story.</p><p>Second paragraph of the story.</p></article>");
    }
  });
  await new Promise((resolve) => site.listen(0, "127.0.0.1", resolve));
  const base = `http://127.0.0.1:${site.address().port}`;
  const port = await reservePort();
  const browser = launch({ args: ["--url", "about:blank", "--allow-downloads"], debugPort: port });
  try {
    const page = await browser.call("navigate.url", { url: `${base}/page` });
    const tabId = page.tabId || (await browser.call("tab.status")).tabId;

    // Reader view toggles an overlay on the page.
    const probe = "document.getElementById('agentgo-reader') ? 'open' : 'closed'";
    assert.equal((await browser.call("browser_ui.reader_mode", { tabId, view: true })).view, "open");
    assert.equal(await cdpEval(port, base, probe), "open");
    assert.equal((await browser.call("browser_ui.reader_mode", { tabId, view: true })).view, "closed");
    assert.equal(await cdpEval(port, base, probe), "closed");
    assert.ok((await browser.call("browser_ui.reader_mode", { tabId })).words > 4, "without view it only reads");

    // Pin and group, as the menu does.
    await browser.call("tab.pin", { tabId });
    await browser.call("tab.group", { tabId, name: "Group 1", color: "#5B8CFF" });
    const listed = (await operator(port, "tabs.list")).tabs.find((tab) => tab.tabId === tabId);
    assert.equal(listed.strip.pinned, true);
    assert.equal(listed.strip.group.name, "Group 1");
    await browser.call("tab.ungroup", { tabId });
    await browser.call("tab.unpin", { tabId });

    // A download is marked and scanned.
    const got = await browser.call("act.download", { url: `${base}/file.txt`, timeout: 15000 });
    const file = got.started.path;
    let scan = null;
    for (let i = 0; i < 100; i += 1) {
      const entry = (await browser.call("download_manager.search", { text: "safe" })).items.find((item) => item.path === file);
      scan = entry && entry.scan;
      if (scan && scan.status !== "scanning") break;
      await delay(500);
    }
    assert.ok(scan, "the download carries a scan result");
    assert.ok(["clean", "unavailable"].includes(scan.status), JSON.stringify(scan));
    const marked = await browser.call("download.quarantine", { path: file });
    assert.equal(marked.marked, true);
    assert.equal(marked.scan, scan.status);
    console.log(`safety_reader_e2e ok (scan: ${scan.status})`);
  } finally {
    site.close();
    await browser.close();
  }
})().catch((error) => {
  console.error(error);
  process.exit(1);
});
