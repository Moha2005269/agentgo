// Extended capability coverage against a real HTTP origin: structured views,
// pointer/keyboard variants, waits, dialogs, telemetry, cookies, artifacts,
// collection workflow, bookmarks and the audit log.
const assert = require("node:assert/strict");
const fs = require("node:fs");
const { launch, serve, rejects, refOf } = require("./harness");

(async () => {
  const site = await serve();
  // Full mode + evaluate grant so the test can also cover runtime.evaluate.
  const browser = launch({ args: ["--url", "about:blank", "--mode", "full", "--allow-evaluate"] });
  const logText = async () => (await browser.call("observe.text")).text;
  try {
    await browser.call("network.observe_requests");
    const opened = await browser.call("navigate.url", { url: site.url("caps.html") });
    assert.equal(opened.loaded, true);
    const page = opened.page;

    const meta = await browser.call("observe.page_metadata");
    assert.equal(meta.title, "AgentGo capability fixture");
    assert.equal(meta.description, "Fixture for extended capability tests");
    assert.equal(meta.lang, "en");

    const tables = await browser.call("observe.tables");
    assert.equal(tables.count, 1);
    assert.deepEqual(tables.tables[0].rows, [["Item", "Price"], ["Widget", "$4.00"], ["Gadget", "$9.50"]]);

    // Pointer variants deliver the real DOM events.
    await browser.call("act.double_click", { ref: refOf(page, "Double target") });
    await browser.call("act.right_click", { ref: refOf(page, "Context target") });
    await browser.call("act.hover", { ref: refOf(page, "Hover target") });
    let text = await logText();
    assert.match(text, /dblclick detail=2 trusted=true/);
    assert.match(text, /contextmenu button=2/);
    assert.match(text, /hovered/);
    await browser.call("pointer.move", { x: 300, y: 300 });
    await browser.call("pointer.hover", { x: 310, y: 300 });
    await browser.call("pointer.double_click", { x: 300, y: 12 });
    await browser.call("pointer.triple_click", { x: 300, y: 12 });
    await browser.call("pointer.right_click", { x: 300, y: 12 });
    await browser.call("pointer.middle_click", { ref: refOf(page, "Context target") });
    text = await logText();
    assert.match(text, /tripleclick/);
    assert.match(text, /mousedown button=1 trusted=true/);

    // Checkbox variants.
    const option = refOf(page, '"Option"');
    assert.equal((await browser.call("act.uncheck", { ref: option })).checked, false);
    assert.equal((await browser.call("act.toggle", { ref: option })).checked, true);
    text = await logText();
    assert.match(text, /option=false\noption=true/);

    // Typing at the focus and a keyboard chord.
    await browser.call("act.click", { ref: refOf(page, "Free text") });
    const typed = await browser.call("act.type", { text: "héllo wörld ✓" });
    assert.match(typed.changes, /value="héllo wörld ✓"/, "the delta shows the new value");
    await browser.call("keyboard.shortcut", { key: "b", modifiers: ["ctrl"] });
    assert.match(await logText(), /shortcut ctrl\+b/);

    // Waits.
    await browser.call("act.click", { ref: refOf(page, "Show panel") });
    assert.equal((await browser.call("wait.selector", { selector: "#panel", timeout: 4000 })).met, true);
    assert.equal((await browser.call("wait.visible", { selector: "#panel-close" })).met, true);
    const panel = await browser.call("observe.page", { filter: "Close panel" });
    await browser.call("act.click", { ref: refOf(panel.page, "Close panel") });
    assert.equal((await browser.call("wait.hidden", { selector: "#panel", timeout: 4000 })).met, true);
    assert.equal((await browser.call("wait.condition", { text: "Capability checks", urlContains: "caps.html" })).met, true);
    await rejects(browser.call("wait.text", { text: "never appears", timeout: 400 }), "timeout");
    for (const method of ["wait.load", "wait.settle", "wait.stable"]) {
      assert.equal((await browser.call(method)).stable, true, method);
    }

    // Dialogs: confirm accepted, prompt answered, confirm dismissed.
    await browser.call("act.click", { ref: refOf(page, "Ask confirm") });
    assert.equal((await browser.call("dialog.list")).dialogs[0].type, "confirm");
    await browser.call("dialog.confirm");
    await browser.call("act.click", { ref: refOf(page, "Ask prompt") });
    await browser.call("dialog.prompt", { text: "Ada" });
    await browser.call("act.click", { ref: refOf(page, "Ask confirm") });
    await browser.call("dialog.close");
    text = await logText();
    assert.match(text, /confirm=true/);
    assert.match(text, /prompt=Ada/);
    assert.match(text, /confirm=false/);
    await rejects(browser.call("dialog.close"), "target.stale");

    // Telemetry rings: console and failed requests (metadata only).
    const consoleEvents = await browser.call("runtime.console");
    assert(consoleEvents.events.some((e) => e.kind === "console" && e.text === "fixture console message"));
    const failures = await browser.call("network.failures");
    assert(failures.events.length >= 1, "the unreachable fetch was recorded as a failure");

    // Cookies: attributes are visible, values never are.
    assert.equal((await browser.call("storage.cookie_set", { name: "agentgo_test", value: "s3cret-value" })).set, true);
    const cookies = await browser.call("storage.cookies_list");
    const cookie = cookies.cookies.find((c) => c.name === "agentgo_test");
    assert.equal(cookie.valueBytes, 12);
    assert(!JSON.stringify(cookies).includes("s3cret-value"));
    assert.equal((await browser.call("storage.cookie_get", { name: "agentgo_test" })).cookies.length, 1);
    await browser.call("storage.cookie_delete", { name: "agentgo_test" });
    assert.equal((await browser.call("storage.cookie_get", { name: "agentgo_test" })).cookies.length, 0);

    // runtime.evaluate is available here only because the operator granted it.
    assert.equal((await browser.call("runtime.evaluate", { expression: "document.cookie.includes('agentgo_test')" })).value, false);
    // The page agent lives in an isolated world the page cannot reach.
    assert.equal((await browser.call("runtime.evaluate", { expression: "typeof __ag" })).value, "undefined");

    // Artifacts: handles with size and hash.
    for (const [method, params, min] of [
      ["observe.full_page_screenshot", {}, 2000],
      ["observe.crop", { ref: refOf(page, "Double target") }, 200],
      ["print.to_pdf", {}, 1000],
      ["page.save_snapshot", {}, 500],
    ]) {
      const artifact = await browser.call(method, params, 45_000);
      assert.equal(fs.statSync(artifact.path).size, artifact.bytes, method);
      assert(artifact.bytes > min, `${method} produced ${artifact.bytes} bytes`);
      assert.match(artifact.sha256, /^[0-9a-f]{64}$/);
    }
    assert.equal(fs.readFileSync((await browser.call("print.to_pdf")).path).subarray(0, 4).toString(), "%PDF");

    // Scroll-and-collect de-duplicates across lazily loaded batches.
    const collected = await browser.call("workflow.collect", { filter: "example.com/item", scrolls: 8 });
    assert.equal(collected.count, 20, JSON.stringify(collected.items.map((i) => i.url)));
    assert.equal(new Set(collected.items.map((i) => i.url)).size, 20);

    // Bookmarks.
    await browser.call("bookmark.add", { url: site.url("caps.html"), title: "Caps" });
    assert.equal((await browser.call("bookmark.list")).items[0].title, "Caps");
    await browser.call("bookmark.remove", { url: site.url("caps.html") });
    assert.equal((await browser.call("bookmark.list")).items.length, 0);

    // Audit log: durable, and typed text never appears in it.
    const audit = await browser.call("audit.list", { limit: 200 });
    assert(audit.items.some((item) => item.event.method === "act.double_click"));
    assert(!JSON.stringify(audit).includes("héllo"));
    assert(!JSON.stringify(audit).includes("s3cret-value"));

    console.log(JSON.stringify({ ok: true, auditEvents: audit.items.length }));
  } catch (error) {
    console.error(error); // report the real failure before cleanup
    process.exitCode = 1;
  } finally {
    const code = await browser.close();
    site.close();
    assert.equal(code, 0, `browser exit code ${code}\n${browser.stderr().slice(-1500)}`);
  }
})().catch((error) => { console.error(error); process.exit(1); });
