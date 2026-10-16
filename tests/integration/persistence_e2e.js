// Profiles really persist: cookies, site storage, history and bookmarks
// survive a restart of a named profile; a second instance of the same
// profile gets an honest throwaway profile instead of corrupting the first;
// throwaway roots are removed at exit.
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { launch, serve, delay } = require("./harness");

const runsDir = path.join(os.tmpdir(), "agentgo-runs");
const runDirOf = (pid) => (fs.existsSync(runsDir) ? fs.readdirSync(runsDir).filter((name) => name.startsWith(`${pid}-`) && !name.endsWith(".lock")) : []);

(async () => {
  const site = await serve();
  const dataRoot = fs.mkdtempSync(path.join(os.tmpdir(), "agentgo-persist-"));
  const args = ["--url", "about:blank", "--profile", "default", "--mode", "full", "--allow-evaluate"];
  const url = site.url("fixture.html");
  let failed = false;
  let second;
  let third;
  try {
    // ---- first run: write state ---------------------------------------------
    const first = launch({ args, dataRoot });
    const info = await first.call("session.info");
    assert.deepEqual(info.profile, { id: "default", ephemeral: false });
    assert.equal(runDirOf(first.child.pid).length, 0, "a durable profile uses no throwaway root");
    await first.call("navigate.url", { url, observe: "none" });
    assert.equal((await first.call("storage.cookie_set", { name: "agentgo_persist", value: "kept-across-restart" })).set, true);
    await first.call("runtime.evaluate", { expression: "localStorage.setItem('agentgo', 'remembered'); document.cookie = 'long=1; max-age=86400'; true" });
    await first.call("bookmark.add", { url, title: "Kept bookmark" });
    assert.equal(await first.close(), 0);
    assert(fs.existsSync(path.join(dataRoot, "profiles", "default", "cef", "content")), "site data lives under the profile directory");

    // ---- second run: everything is still there -------------------------------
    second = launch({ args, dataRoot });
    assert.equal((await second.call("session.info")).profile.ephemeral, false);
    const history = await second.call("history.list");
    assert.equal(history.items.find((item) => item.url === url)?.title, "AgentGo fixture — interaction page");
    assert.equal((await second.call("bookmark.list")).items[0].title, "Kept bookmark");
    await second.call("navigate.url", { url, observe: "none" });
    const cookies = (await second.call("storage.cookies_list")).cookies.map((cookie) => cookie.name).sort();
    assert.deepEqual(cookies, ["agentgo_persist", "long"], "session and persistent cookies both survive");
    assert.equal((await second.call("runtime.evaluate", { expression: "localStorage.getItem('agentgo')" })).value, "remembered");

    // ---- a concurrent launch of the same profile is isolated and honest ------
    third = launch({ args, dataRoot });
    const thirdInfo = await third.call("session.info");
    assert.deepEqual(thirdInfo.profile, { id: "default", ephemeral: true });
    assert.notEqual(thirdInfo.instanceId, (await second.call("session.info")).instanceId);
    await third.call("navigate.url", { url, observe: "none" });
    assert.equal((await third.call("storage.cookies_list")).cookies.length, 0, "the throwaway profile starts clean");
    const thirdPid = third.child.pid;
    assert.equal(runDirOf(thirdPid).length, 1);
    // Private history stays out of the durable store.
    await third.call("navigate.url", { url: site.url("caps.html"), observe: "none" });
    assert.equal(await third.close(), 0);
    third = null;
    // Chromium holds a lock in its own root until the process is gone, so a
    // throwaway root is removed at exit when possible and otherwise swept by
    // the next launch.
    await delay(500);
    const sweeper = launch({ args: ["--url", "about:blank"], dataRoot });
    await sweeper.call("api.health");
    assert.equal(runDirOf(thirdPid).length, 0, "the stale throwaway root is swept by the next launch");
    assert.equal(await sweeper.close(), 0);
    assert(!(await second.call("history.list")).items.some((item) => item.url.endsWith("caps.html")), "private browsing left no durable history");

    assert.equal(await second.close(), 0);
    second = null;
    console.log(JSON.stringify({ ok: true }));
  } catch (error) {
    console.error(error);
    failed = true;
  } finally {
    for (const browser of [second, third]) if (browser) await browser.close();
    site.close();
    fs.rmSync(dataRoot, { recursive: true, force: true });
    process.exitCode = failed ? 1 : 0;
  }
})();
