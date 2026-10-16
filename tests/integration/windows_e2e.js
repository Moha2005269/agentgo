// More windows and the tab strip: secondary windows (never taking the
// foreground), detach/attach, split, workspaces, presentation mode, the
// agent overlay and the person's prompt, pin/group/sort/move, discard and
// restore, freezing, duplicates, frames and the address bar's suggestions.
const assert = require("node:assert/strict");
const { launch, operator, cdpEval, reservePort, serve, fixture, rejects, delay } = require("./harness");

(async () => {
  const site = await serve();
  const port = await reservePort();
  const browser = launch({ args: ["--url", "about:blank"], debugPort: port });
  const op = (method, params) => operator(port, method, params);
  const strip = () => cdpEval(port, "chrome.html", "[...document.querySelectorAll('#tabs .tab')].map(t => t.dataset.tabId + (t.classList.contains('pinned') ? ':pin' : '') + (t.dataset.group ? ':' + t.dataset.group : '')).join(',')");
  try {
    const first = await browser.call("navigate.url", { url: site.url("fixture.html") });
    const main = first.tabId || (await browser.call("tab.status")).tabId;

    // A second window with its own tab.
    const made = await browser.call("window.create", { url: fixture("tools.html"), width: 640, height: 480 });
    assert.match(made.windowId, /^window-/);
    let list = await browser.call("window.list");
    assert.equal(list.windows.length, 2);
    assert.deepEqual(list.windows.find((w) => w.windowId === made.windowId).tabs, [made.tabId]);
    assert.ok(!(await strip()).includes(made.tabId), "a tab of another window is not in the main strip");
    assert.ok((await browser.call("observe.text", { tabId: made.tabId })).text.length > 0, "its page works");
    const moved = await browser.call("window.move", { windowId: made.windowId, x: 50, y: 60, width: 700 });
    assert.equal(moved.bounds.width, 700);
    await rejects(browser.call("window.focus", { windowId: made.windowId }), "policy.denied");
    assert.equal((await op("window.focus", { windowId: made.windowId })).active, true);
    await browser.call("window.attach_tab", { tabId: made.tabId });
    for (let i = 0; i < 20 && (await browser.call("window.list")).windows.length > 1; i += 1) await delay(150);
    assert.equal((await browser.call("window.list")).windows.length, 1, "an empty window closes");
    assert.ok((await strip()).includes(made.tabId));

    // Detach, then move the main window only with the person's approval.
    const second = await browser.call("tabs.create", { url: site.url("fixture.html") });
    const detached = await browser.call("window.detach_tab", { tabId: second.tabId });
    assert.equal((await browser.call("window.list")).windows.length, 2);
    const gate = await rejects(browser.call("window.move", { x: 10, y: 10, width: 900, height: 700 }), "approval.required");
    await op("agent.approval.respond", { approvalId: gate.data.approvalId, approve: true });
    assert.equal((await browser.call("window.move", { x: 10, y: 10, width: 900, height: 700, approvalId: gate.data.approvalId })).bounds.width, 900);
    await browser.call("window.attach_tab", { tabId: detached.tabId });

    // Split: the main window left, the tab's window right.
    const splitGate = await rejects(browser.call("window.split", { tabId: second.tabId }), "approval.required");
    await op("agent.approval.respond", { approvalId: splitGate.data.approvalId, approve: true });
    const split = await browser.call("window.split", { tabId: second.tabId, approvalId: splitGate.data.approvalId });
    assert.equal(split.left, "main");
    const windows = (await browser.call("window.list")).windows;
    const left = windows.find((w) => w.windowId === "main").bounds;
    const right = windows.find((w) => w.windowId === split.right).bounds;
    assert.ok(right.x >= left.x + left.width - 2, JSON.stringify(windows));

    // Workspaces.
    assert.equal((await browser.call("window.workspace", { action: "save", name: "two" })).windows, 2);
    assert.deepEqual((await browser.call("window.workspace", { action: "list" })).workspaces, ["two"]);
    await browser.call("window.attach_tab", { tabId: second.tabId });
    const restored = await browser.call("window.workspace", { action: "restore", name: "two" });
    assert.ok(restored.tabs.length >= 2);
    assert.equal((await browser.call("window.list")).windows.length, 2);
    await browser.call("window.workspace", { action: "delete", name: "two" });

    // Popup window.
    const popup = await browser.call("tab.open_popup", { url: "about:blank", width: 400, height: 500 });
    assert.ok(popup.windowId);

    // Presentation mode, overlay and prompt.
    assert.equal((await browser.call("window.presentation_mode", { on: true })).presentation, true);
    assert.equal((await browser.call("window.presentation_mode", { on: false })).presentation, false);
    assert.equal((await browser.call("window.agent_overlay", { text: "Comparing prices" })).shown, true);
    await delay(200);
    assert.equal(await cdpEval(port, "chrome.html", "document.querySelector('#agentOverlay').hidden === false && overlayState.note"), "Comparing prices");
    await browser.call("window.always_visible_prompt", { on: true });
    await browser.call("events.subscribe");
    await cdpEval(port, "chrome.html", "(() => { const i = document.querySelector('#personPromptText'); i.value = 'use the cheaper one'; document.querySelector('#personPrompt').requestSubmit(); return true; })()");
    await delay(300);
    assert.deepEqual((await browser.call("window.always_visible_prompt", { read: true })).messages.map((m) => m.text), ["use the cheaper one"]);

    // Tab strip: pin, group, move, sort.
    const a = await browser.call("tabs.create", { url: site.url("tools.html") });
    const b = await browser.call("tabs.create", { url: site.url("fixture.html") });
    await browser.call("tab.pin", { tabId: b.tabId });
    await delay(200);
    assert.ok((await strip()).startsWith(`${b.tabId}:pin`), await strip());
    await browser.call("tab.unpin", { tabId: b.tabId });
    await browser.call("tab.group", { tabIds: [a.tabId, b.tabId], name: "Research", color: "#32C878" });
    await delay(200);
    assert.ok((await strip()).includes(`${a.tabId}:Research`));
    assert.equal(await cdpEval(port, "chrome.html", "document.querySelector('.tab-group')?.textContent"), "Research");
    await browser.call("tab.ungroup", { tabIds: [a.tabId, b.tabId] });
    const movedTab = await browser.call("tab.move", { tabId: a.tabId, index: 0 });
    assert.equal(movedTab.order[0], a.tabId);
    const sorted = await browser.call("tab.sort", { by: "url" });
    assert.ok(Array.isArray(sorted.order));
    await rejects(browser.call("tab.sort", { by: "color" }), "invalid.params");

    // Discard and restore; duplicates; freezing.
    const discarded = await browser.call("tab.discard", { tabId: a.tabId });
    assert.equal(discarded.discarded, true);
    await browser.call("wait.load", { tabId: a.tabId });
    assert.equal((await browser.call("tab.status", { tabId: a.tabId })).url, "about:blank");
    assert.equal((await browser.call("tab.restore", { tabId: a.tabId })).restored, true);
    await browser.call("wait.load", { tabId: a.tabId });
    assert.match((await browser.call("tab.status", { tabId: a.tabId })).url, /tools\.html/);
    const dupes = await browser.call("tab.suspend_duplicates");
    assert.ok(dupes.suspended.length >= 1, JSON.stringify(dupes));
    const frozen = await browser.call("tab.freeze_all_background");
    assert.ok(Array.isArray(frozen.frozen));

    // Frames.
    await browser.call("navigate.url", { tabId: a.tabId, url: site.url("tools.html") });
    const selected = await browser.call("frame.select", { tabId: a.tabId, name: "inner" });
    assert.equal(selected.selected.name, "inner");
    assert.ok((await browser.call("frame.focus", { tabId: a.tabId })).focused);
    assert.ok((await browser.call("focus.frame", { tabId: a.tabId, index: 0 })).focused);
    await browser.call("frame.navigate", { tabId: a.tabId, url: site.url("minimal.html") });
    await delay(500);
    const frames = await browser.call("frame.list", { tabId: a.tabId });
    assert.ok(JSON.stringify(frames).includes("minimal.html") || frames.count >= 1);
    await rejects(browser.call("frame.select", { tabId: a.tabId, name: "nope" }), "target.stale");

    // The address bar.
    await browser.call("bookmark.add", { url: site.url("tools.html"), title: "Tools fixture" }).catch(() => {});
    const typed = await browser.call("omnibox.type", { text: "tools" });
    assert.ok(typed.suggestions.some((s) => s.kind === "search"));
    assert.ok(typed.suggestions.some((s) => s.url.includes("tools.html")), JSON.stringify(typed.suggestions));
    await delay(400);
    assert.equal(await cdpEval(port, "chrome.html", "document.querySelector('#address').value"), "tools");
    assert.ok((await browser.call("omnibox.suggestions", { text: "example.com" })).suggestions[0].kind === "address");
    const pick = typed.suggestions.find((s) => s.kind !== "search" && s.kind !== "tab");
    if (pick) {
      const opened = await browser.call("omnibox.select_suggestion", { text: "tools", index: pick.index, tabId: main });
      assert.match(opened.url, /tools\.html/);
    }
    assert.equal((await browser.call("omnibox.clear")).text, "");
    assert.ok((await browser.call("window.monitor_list")).monitors.length >= 1);
    console.log("windows_e2e: PASS");
  } finally {
    await browser.close();
    site.close();
  }
})().catch((error) => {
  console.error(error);
  process.exit(1);
});
