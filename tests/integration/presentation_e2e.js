// An agent-started browser must never interrupt the person: its window opens
// behind theirs, never becomes the active window, yet stays a normal window
// with a taskbar button whose pages keep running. The agent works with its own
// (DevTools) pointer and keyboard, also in tabs that are not on screen. A decoy
// window stands in for whatever the person is working in.
const assert = require("node:assert/strict");
const path = require("node:path");
const { spawn, execFileSync } = require("node:child_process");
const { launch, delay, repo, reservePort, operator, fixture, rejects, refOf } = require("./harness");

const tool = path.join(repo, "target", "debug", "agentgo-tool.exe");
const windows = () => JSON.parse(execFileSync(tool, ["probe", "windows"], { encoding: "utf8" })).windows;
const big = (w) => w.visible && w.rect.right - w.rect.left > 600;

async function startDecoy() {
  const decoy = spawn(tool, ["probe", "decoy", "--cover"], { stdio: ["ignore", "pipe", "inherit"] });
  await new Promise((resolve) => decoy.stdout.on("data", (d) => String(d).includes("decoy-ready") && resolve()));
  await delay(300);
  return decoy;
}

// The browser's main window and the decoy, as an outside observer sees them.
function observe(browser, decoy) {
  const all = windows();
  const ours = all.find((w) => w.pid === browser.child.pid && big(w));
  const theirs = all.find((w) => w.pid === decoy.pid && big(w));
  assert(ours, "the browser window is not visible");
  assert(theirs, "the decoy window is gone");
  return { ours, theirs };
}

(async () => {
  let failed = false;
  const port = await reservePort();
  const decoy = await startDecoy();
  // On the real desktop Windows may not let the decoy become active at all
  // (another program holds the foreground); the browser must then leave the
  // decoy exactly as it was.
  const decoyBefore = windows().find((w) => w.pid === decoy.pid && big(w));
  const decoyWasActive = decoyBefore?.activeInQueue === true;
  const decoyWasForeground = decoyBefore?.foreground === true;
  // On the private desktop nothing else runs, so "active in its queue" is the
  // same as the foreground; on the person's desktop only the foreground counts.
  const shared = process.env.AGENTGO_TEST_VISIBLE === "1";
  const browser = launch({ args: ["--url", fixture("live.html"), "--mode", "full", "--allow-evaluate"], debugPort: port });
  const quiet = (when) => {
    const { ours, theirs } = observe(browser, decoy);
    assert.equal(ours.foreground, false, `the browser took the foreground ${when}`);
    // On the person's desktop they may switch windows during the run.
    if (!shared) assert.equal(theirs.foreground, decoyWasForeground, `the person's window lost the foreground ${when}; foreground now: ${JSON.stringify(windows().filter((w) => w.foreground).map((w) => [w.pid, w.title, w.class]))} (browser pid ${browser.child.pid})`);
    if (!shared) assert.equal(ours.activeInQueue, false, `the browser activated itself ${when}`);
    if (!shared) assert.equal(ours.focusInQueue, false, `the browser took the keyboard focus ${when}`);
    if (!shared) assert.equal(theirs.activeInQueue, decoyWasActive, `the person's window lost activation ${when}; active now: ${JSON.stringify(windows().filter((w) => w.activeInQueue).map((w) => [w.pid, w.title]))}`);
    assert(ours.z > theirs.z, `the browser moved in front of the person's window ${when}`);
    return ours;
  };
  try {
    await browser.call("browser.health");
    await delay(1200);
    const ours = quiet("at launch");
    assert.equal(ours.minimized, false);
    assert.equal(ours.taskbar, true, "the window has no taskbar button");
    assert.match(ours.title, /AgentGo$/);

    const status = await browser.call("window.status");
    assert.equal(status.visible, true);
    assert.equal(status.active, false);
    assert.equal(status.minimized, false);
    assert.equal(status.presentation, "background");
    assert.equal(status.humanActive, false);
    assert.equal(status.agentTabs, "auto");
    assert.equal(status.activeTabId, "content-1");
    assert(status.bounds.width > 600 && status.bounds.height > 400);

    // Covered by the decoy, the page still renders, runs timers and can be
    // photographed.
    const sample = async (tabId) => (await browser.call("runtime.evaluate", { tabId, expression: "[window.frames_seen, window.timers_seen]" })).value;
    const live = async (tabId, what) => {
      const a = await sample(tabId);
      await delay(1000);
      const b = await sample(tabId);
      assert(b[0] - a[0] >= 10, `${what} stopped rendering: ${b[0] - a[0]} frames in 1 s`);
      assert(b[1] - a[1] >= 10, `${what} timers are throttled: ${b[1] - a[1]} ticks in 1 s`);
    };
    await live("content-1", "the covered page");
    assert((await browser.call("observe.screenshot", {})).path, "no screenshot of the covered page");

    // The agent's own pointer and keyboard work on the covered page and the
    // page believes it has the focus.
    let view = await browser.call("observe.page");
    await browser.call("act.click", { ref: refOf(view.page, "Press me") });
    assert.match((await browser.call("observe.text")).text, /pressed/);
    await browser.call("act.fill", { ref: refOf(view.page, "Your name"), text: "Ada" });
    assert.match((await browser.call("observe.text")).text, /typed Ada/);
    assert.equal((await browser.call("runtime.evaluate", { expression: "document.hasFocus()" })).value, true);
    quiet("after clicking and typing");

    // The person told the browser to keep agent tabs out of the way.
    await rejects(browser.call("settings.set", { settings: { agentTabs: "follow" } }), "policy.denied");
    await operator(port, "settings.set", { settings: { agentTabs: "background" } });
    const created = await browser.call("tabs.create", { url: fixture("live.html"), _byAgent: false });
    assert.equal(created.active, false, "an agent passed itself off as the person");
    assert.equal((await browser.call("window.status")).activeTabId, "content-1");
    const tabId = created.tabId;
    await browser.call("wait.load", { tabId });
    const hidden = await browser.call("tabs.activate", { tabId });
    assert.equal(hidden.applied, false);
    assert.equal(hidden.activeTabId, "content-1");

    // A tab that is not on screen is fully usable.
    await live(tabId, "the background tab");
    view = await browser.call("observe.page", { tabId });
    await browser.call("act.click", { tabId, ref: refOf(view.page, "Press me") });
    assert.match((await browser.call("observe.text", { tabId })).text, /pressed/);
    await browser.call("act.fill", { tabId, ref: refOf(view.page, "Your name"), text: "Bo" });
    assert.match((await browser.call("observe.text", { tabId })).text, /typed Bo/);
    assert((await browser.call("observe.screenshot", { tabId })).path, "no screenshot of the background tab");
    // ... and the visible tab was not disturbed by any of it.
    assert.match((await browser.call("observe.text", { tabId: "content-1" })).text, /typed Ada/);
    quiet("after working in a background tab");

    await operator(port, "settings.set", { settings: { agentTabs: "follow" } });
    const shown = await browser.call("tabs.activate", { tabId });
    assert.equal(shown.applied, true);
    assert.equal(shown.activeTabId, tabId);
    await delay(300);
    quiet("after switching tabs");
    await browser.call("navigate.url", { tabId, url: fixture("fixture.html") });
    quiet("after navigating");
  } catch (error) {
    console.error(error);
    failed = true;
  }
  let code = await browser.close();

  // A launch the person made on purpose opens in front, as any program does.
  const front = launch({ args: ["--url", "about:blank", "--foreground"] });
  try {
    await front.call("browser.health");
    await delay(1200);
    assert.equal((await front.call("window.status")).presentation, "foreground");
    const { ours, theirs } = observe(front, decoy);
    // Windows lets a launch take the foreground only when it comes from the
    // program the person is using; a test runner is not that program.
    if (!shared) assert.equal(ours.activeInQueue, true, "a foreground launch did not activate its window");
    if (!shared) assert(ours.z < theirs.z, "a foreground launch opened behind the other window");
  } catch (error) {
    console.error(error);
    failed = true;
  }
  code ||= await front.close();
  decoy.kill();
  if (!failed) console.log(JSON.stringify({ ok: true }));
  process.exitCode = failed || code !== 0 ? 1 : 0;
})();
