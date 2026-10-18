// Everyday abilities an agent needs beyond click and type: dragging (pointer
// drags and HTML drag-and-drop), a clipboard of its own, find in page, zoom,
// and search results as records in one call.
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const http = require("node:http");
const { launch, serve, rejects, refOf, reservePort, delay, operator } = require("./harness");

(async () => {
  const site = await serve();
  const port = await reservePort();
  const browser = launch({ args: ["--url", "about:blank", "--mode", "full", "--allow-evaluate"], debugPort: port });
  const text = async () => (await browser.call("observe.text", { max: 6000 })).text;
  let failed = false;
  try {
    const view = await browser.call("navigate.url", { url: site.url("daily.html") });

    // Pointer drag: the knob follows the agent's pointer and stays where it
    // is released.
    const slid = await browser.call("act.drag", { ref: refOf(view.page, "Slider knob"), toX: 300, toY: 90 });
    assert.equal(slid.dragged, "pointer");
    const knob = Number((await text()).match(/knob at (\d+)/)[1]);
    assert(knob > 200 && knob <= 380, `the knob did not follow the drag: ${knob}`);

    // HTML drag-and-drop between two elements, with the dragged data intact.
    const dropped = await browser.call("act.drag", { ref: refOf(view.page, "Report card"), toRef: refOf(view.page, "Archive zone") });
    assert.equal(dropped.dragged, "drag-and-drop");
    assert.match(await text(), /dropped report-card/);
    await rejects(browser.call("act.drag", { ref: refOf(view.page, "Report card") }), "invalid.params");

    // The agent's clipboard: copy what is selected, paste it elsewhere. The
    // Windows clipboard is never involved.
    const found = await browser.call("page.find", { text: "copies this exact" });
    assert.equal(found.count, 1);
    const copied = await browser.call("clipboard.copy");
    assert.equal(copied.text, "copies this exact");
    assert.equal((await browser.call("clipboard.read")).text, "copies this exact");
    await browser.call("act.click", { ref: refOf(view.page, "Paste target") });
    assert.equal((await browser.call("clipboard.paste")).pasted, 17);
    assert.match(await text(), /pasted copies this exact/);
    await browser.call("clipboard.write", { text: " twice" });
    await browser.call("clipboard.paste");
    assert.match(await text(), /pasted copies this exact twice/);
    const osClipboard = (await browser.call("runtime.evaluate", { expression: "navigator.clipboard.readText().catch(() => 'unreadable')" })).value;
    assert.notEqual(osClipboard, "copies this exact twice");

    // Find in page: every match is counted and the chosen one is scrolled to.
    const needles = await browser.call("page.find", { text: "NEEDLE", index: 1 });
    assert.equal(needles.count, 2);
    assert.equal(needles.index, 1);
    assert.match(needles.matches[0].context, /second needle sits here/i);
    assert((await browser.call("runtime.evaluate", { expression: "scrollY" })).value > 1000, "the match was not scrolled into view");
    assert.equal((await browser.call("page.find", { text: "not on this page" })).count, 0);

    // Zoom is a real browser zoom: the layout viewport shrinks.
    const width = async () => (await browser.call("runtime.evaluate", { expression: "innerWidth" })).value;
    const normal = await width();
    assert.equal((await browser.call("page.zoom")).percent, 100);
    assert.equal((await browser.call("page.zoom", { percent: 200 })).percent, 200);
    const zoomed = await width();
    assert(Math.abs(zoomed - normal / 2) <= 2, `zoom did not change the layout: ${normal} -> ${zoomed}`);
    assert.equal((await browser.call("page.zoom")).percent, 200);
    await browser.call("page.zoom", { percent: 100 });
    await rejects(browser.call("page.zoom", { percent: 5000 }), "invalid.params");

    // Search results as records, in one request.
    const search = await browser.call("workflow.search_extract", { query: "agent browser", url: site.url("serp.html") });
    assert.deepEqual(
      search.results.map((r) => [r.rank, r.title, r.url]),
      [
        [1, "First result title", "https://example.com/first"],
        [2, "Second result title", "https://example.org/second"],
        [3, "Third result title", "https://example.net/third"],
      ],
    );
    assert.match(search.results[0].snippet, /first snippet explains/);
    assert.equal(typeof search.results[0].ref, "number");
    assert.equal((await browser.call("workflow.search_extract", { query: "agent browser", url: site.url("serp.html"), limit: 2 })).results.length, 2);

    // A site that never answers: every call still returns within its time.
    const silent = http.createServer(() => {});
    await new Promise((resolve) => silent.listen(0, "127.0.0.1", resolve));
    try {
      const timed = async (what, promise, limit) => {
        const started = Date.now();
        const outcome = await promise.then((r) => r, (e) => e);
        assert(Date.now() - started < limit, `${what} took ${Date.now() - started} ms`);
        return outcome;
      };
      const pending = await timed("navigate.url", browser.call("navigate.url", { url: `http://127.0.0.1:${silent.address().port}/never`, timeout: 2000 }), 4500);
      assert.deepEqual([pending.loaded, pending.committed], [false, false]);
      assert.equal((await timed("observe.page", browser.call("observe.page"), 6000)).code, "timeout");
      assert.equal((await timed("wait.load", browser.call("wait.load", { timeout: 1000 }), 2500)).loaded, false);
      await browser.call("tab.stop");
      await timed("observe.page after stop", browser.call("observe.page"), 6000);
    } finally {
      silent.closeAllConnections();
      silent.close();
    }

    // The person can watch the agent: its own pointer and live typing are
    // drawn on the page, and typing arrives key by key. Switched off, the page
    // is left untouched and text arrives at once.
    const counter = "window.__inputs = 0; document.querySelector('#target').addEventListener('input', () => { window.__inputs += 1; }); true";
    const stage = async () => (await browser.call("runtime.evaluate", { expression: "[Boolean(document.querySelector('agentgo-stage')), window.__inputs]" })).value;
    let page = await browser.call("navigate.url", { url: site.url("daily.html") });
    await browser.call("runtime.evaluate", { expression: counter });
    await browser.call("act.fill", { ref: refOf(page.page, "Paste target"), text: "watch" });
    assert.deepEqual(await stage(), [true, 5]);
    // The overlay takes no input and is invisible to observation.
    assert.equal((await browser.call("runtime.evaluate", { expression: "getComputedStyle(document.querySelector('agentgo-stage')).pointerEvents" })).value, "none");
    assert.equal((await browser.call("observe.page")).page.includes("Agent is typing"), false);
    assert.equal((await browser.call("observe.text")).text.includes("Agent is typing"), false);
    // Key presses, scrolling and waits are shown too.
    for (const step of [["act.press", { key: "End" }], ["act.scroll", { dy: 200 }], ["wait.text", { text: "Daily fixture", timeout: 2000 }]]) {
      await browser.call("navigate.url", { url: site.url("daily.html"), observe: "none" });
      // The navigation notice drew the overlay; take it away.
      await browser.call("runtime.evaluate", { expression: "document.querySelector('agentgo-stage')?.remove(); true" });
      assert.equal((await stage())[0], false);
      await browser.call(...step);
      assert.equal((await stage())[0], true, `${step[0]} drew nothing`);
    }
    // Only the person can hide what an agent does.
    await rejects(browser.call("settings.set", { settings: { agentAnimations: false } }), "policy.denied");
    await operator(port, "settings.set", { settings: { agentAnimations: false } });
    page = await browser.call("navigate.url", { url: site.url("daily.html") });
    await browser.call("runtime.evaluate", { expression: counter });
    await browser.call("act.fill", { ref: refOf(page.page, "Paste target"), text: "quiet" });
    await browser.call("act.click", { ref: refOf(page.page, "Paste target") });
    assert.deepEqual(await stage(), [false, 1]);
    await operator(port, "settings.set", { settings: { agentAnimations: true } });

    // The decision interface (what GoMini, the local choice model, is fed):
    // a subgoal becomes a record of candidates; a choice runs only against the
    // state it was made for and only with text the planner supplied.
    assert.deepEqual([(await browser.call("decision.status")).available, (await browser.call("decision.status")).enabled], [false, false]);
    await rejects(browser.call("settings.set", { settings: { gominiEnabled: true } }), "policy.denied");
    await browser.call("navigate.url", { url: site.url("daily.html") });
    const values = { note: "chosen by candidate" };
    const record = await browser.call("decision.record", { subgoal: "write the note into the paste target", values, completion: "the page says pasted" });
    assert.equal(record.schema, "agentgo.gomini.decision/1");
    const fillTarget = record.candidates.find((c) => c.op === "fill" && /Paste target/.test(c.target));
    assert(fillTarget, "the field is not among the candidates");
    assert(["scroll_down", "back", "wait", "done", "escalate"].every((id) => record.candidates.some((c) => c.id === id)));
    await rejects(browser.call("decision.apply", { choose: fillTarget.id, value: "password", values }), "invalid.params");
    await rejects(browser.call("decision.apply", { choose: "r999999", values }), "invalid.params");
    const applied = await browser.call("decision.apply", { choose: fillTarget.id, value: "note", values });
    assert.equal(applied.ran, "act.fill");
    assert.match(await text(), /pasted chosen by candidate/);
    // The page changed, so the old candidates are no longer valid.
    await rejects(browser.call("decision.apply", { choose: fillTarget.id, value: "note", values }), "target.stale");
    await browser.call("decision.record", { subgoal: "finish" });
    assert.deepEqual(await browser.call("decision.apply", { choose: "done" }), { ran: null, control: "done" });
    await rejects(browser.call("decision.record", {}), "invalid.params");

    // Recording and replay: a recording made on one load replays on another,
    // where every reference number is different, and can be fed other data.
    let flow = await browser.call("navigate.url", { url: site.url("daily.html") });
    assert.equal((await browser.call("workflow.record", { name: "paste-note" })).recording, true);
    await browser.call("act.fill", { ref: refOf(flow.page, "Paste target"), text: "first run" });
    await browser.call("workflow.pause");
    await browser.call("act.fill", { ref: refOf(flow.page, "Paste target"), text: "not recorded" });
    await browser.call("workflow.resume");
    await browser.call("act.click", { ref: refOf(flow.page, "Archive zone") });
    const recorded = await browser.call("workflow.record", { stop: true });
    assert.equal(recorded.steps, 2);
    flow = await browser.call("navigate.url", { url: site.url("daily.html") });
    const replayed = await browser.call("workflow.replay", { name: "paste-note", values: { "first run": "second run" } });
    assert.equal(replayed.completed, 2);
    assert.match(await text(), /pasted second run/);
    // A target that is gone stops the replay with the step that failed.
    await browser.call("navigate.url", { url: site.url("serp.html") });
    const gone = await browser.call("workflow.replay", { name: "paste-note" }).catch((error) => error);
    assert.equal(gone.code, "target.stale");
    assert.equal(gone.data.completed, 0);
    await browser.call("workflow.disable", { name: "paste-note" });
    await rejects(browser.call("workflow.replay", { name: "paste-note" }), "policy.denied");
    await rejects(browser.call("workflow.record", { name: "../escape" }), "invalid.params");

    // Made possible: pen input, permissions the person grants, passkeys,
    // recording, DevTools, and leaving the browser only with approval.
    await browser.call("navigate.url", { url: site.url("daily.html") });
    await browser.call("pen.down", { x: 200, y: 300, pressure: 0.7 });
    await browser.call("pen.up", { x: 200, y: 300 });
    assert.match(await text(), /pen at 200 pressure 0\.70/);
    // A real request: a refusal is error code 1; anything else means the site was allowed.
    const askNotify = "new Promise((done) => navigator.geolocation.getCurrentPosition(() => done('granted'), (e) => done(e.code === 1 ? 'denied' : 'granted'), { timeout: 3000 }))";
    assert.equal((await browser.call("runtime.evaluate", { expression: askNotify })).value, "denied");
    await rejects(browser.call("permission.grant", { origin: site.url(""), kinds: ["notifications"] }), "policy.denied");
    await rejects(browser.call("settings.set", { settings: { sitePermissions: {} } }), "policy.denied");
    const origin = new URL(site.url("")).origin;
    assert.deepEqual((await operator(port, "permission.grant", { origin, kinds: ["geolocation"] })).sites[origin], ["geolocation"]);
    await rejects(operator(port, "permission.grant", { origin, kinds: ["teleport"] }), "invalid.params");
    await browser.call("navigate.url", { url: site.url("daily.html") });
    assert.equal((await browser.call("runtime.evaluate", { expression: askNotify })).value, "granted");
    assert.deepEqual((await operator(port, "permission.reset", { origin })).sites, {});
    // A virtual passkey device answers sign-in prompts: the person approves it.
    const ask = await rejects(browser.call("device.web_authn"), "approval.required");
    await operator(port, "agent.approval.respond", { approvalId: ask.data.approvalId, approve: true });
    const authenticator = await browser.call("device.web_authn", { approvalId: ask.data.approvalId });
    assert.equal(typeof authenticator.authenticatorId, "string");
    await browser.call("device.web_authn", { remove: authenticator.authenticatorId });
    const video = await browser.call("observe.record_video", { seconds: 1, fps: 3 });
    assert.equal(video.frames, 3);
    assert(fs.existsSync(video.player) && fs.existsSync(video.first));
    assert.equal((await browser.call("debug.devtools_open")).open, true);
    assert.equal((await browser.call("debug.devtools_close")).open, false);
    for (const [method, params] of [
      ["navigate.external", { url: "mailto:someone@example.com" }],
      ["native.external_app", { program: "notepad.exe" }],
      ["native.open_with", { path: "C:/Windows/win.ini", program: "notepad.exe" }],
    ]) await rejects(browser.call(method, params), "approval.required");
    await rejects(browser.call("navigate.external", { url: "https://example.com" }), "invalid.params");
    await rejects(browser.call("download.open", { id: 999999 }), "invalid.params");

    // Touch, held buttons and held keys.
    await browser.call("navigate.url", { url: site.url("daily.html") });
    await browser.call("touch.start", { x: 100, y: 400 });
    await browser.call("touch.end");
    await browser.call("touch.swipe", { x: 100, y: 400, toX: 300, toY: 400 });
    await browser.call("touch.rotate", { x: 300, y: 400, degrees: 30 });
    assert.match(await text(), /start1,end,start1,end,start1,start2,end,end/);
    assert.equal((await browser.call("touch.pinch", { x: 200, y: 300, scale: 1.5 })).scale, 1.5);
    await browser.call("touch.long_press", { x: 100, y: 400, ms: 300 });
    await browser.call("keyboard.key_down", { key: "Shift" });
    assert.match(await text(), /shift held/);
    assert.deepEqual((await browser.call("pointer.modifiers")).keys, ["Shift"]);
    await browser.call("keyboard.key_up", { key: "Shift" });
    assert.match(await text(), /shift released/);
    const knobView = await browser.call("observe.page");
    const knobBox = await browser.call("runtime.get_rect", { selector: "#knob" });
    await browser.call("pointer.drag_start", { x: knobBox.x + 10, y: knobBox.y + 10 });
    assert.equal((await browser.call("pointer.button_state")).left, true);
    await browser.call("pointer.drag_move", { x: knobBox.x + 150, y: knobBox.y + 10 });
    await browser.call("pointer.drag_end", { x: knobBox.x + 150, y: knobBox.y + 10 });
    assert.equal((await browser.call("pointer.button_state")).left, false);
    assert(Number((await text()).match(/knob at (\d+)/)[1]) > 100, "the held-button drag did not move the knob");
    await rejects(browser.call("keyboard.key_down", { key: "Hyper" }), "invalid.params");
    void knobView;

    // A page that cannot load says so, to the person and to the agent.
    const dead = `http://127.0.0.1:${await reservePort()}/gone`;
    const failedLoad = await browser.call("navigate.url", { url: dead, observe: "text" });
    assert.match(failedLoad.loadError, /CONNECTION_REFUSED/);
    assert.match(failedLoad.text, /This page did not load[\s\S]*refused the connection/);
    assert.match((await browser.call("tabs.status")).url, /^agentgo:\/\/shell\/error\.html\?url=/);
    // The error page is not a trusted surface.
    assert.notEqual((await browser.call("runtime.evaluate", { expression: "fetch('agentgo://shell/__bridge?method=settings.get&params=%7B%7D').then(r => r.status, () => 0)" })).value, 200);
  } catch (error) {
    console.error(error);
    failed = true;
  }
  let code = await browser.close();

  // Session restore: a launch that names no page reopens what was open.
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "agentgo-session-"));
  try {
    const first = launch({ args: ["--profile", "default", "--url", site.url("daily.html")], dataRoot: root });
    await first.call("tabs.create", { url: site.url("serp.html") });
    await first.call("wait.load", {});
    await delay(300);
    code ||= await first.close();
    const second = launch({ args: ["--profile", "default", "--restore"], dataRoot: root });
    await second.call("wait.load", {});
    const restored = (await second.call("tabs.list")).tabs.map((tab) => tab.url);
    assert.deepEqual(restored, [site.url("daily.html"), site.url("serp.html")]);
    code ||= await second.close();
    // Without the flag an agent's launch starts clean (here: the page it names).
    const third = launch({ args: ["--profile", "default", "--url", "about:blank"], dataRoot: root });
    assert.equal((await third.call("tabs.list")).tabs.length, 1);
    code ||= await third.close();
    if (!failed) console.log(JSON.stringify({ ok: true }));
  } catch (error) {
    console.error(error);
    failed = true;
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
    site.close();
    process.exitCode = failed || code !== 0 ? 1 : 0;
  }
})();
