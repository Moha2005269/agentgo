// Browser product core over the stdio protocol: health, versions, typed
// errors, tabs, window, settings, history, profiles, downloads, raw
// pointer/keyboard input, and the durable instance record.
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { launch, operator, reservePort, fixture, rejects, delay } = require("./harness");

const page = fixture("fixture.html");
const downloadPage = fixture("download.html");
const landing = "agentgo://shell/index.html?embedded=1";

async function until(read, done, what, timeoutMs = 8000) {
  const started = Date.now();
  for (;;) {
    const value = await read();
    if (done(value)) return value;
    if (Date.now() - started > timeoutMs) throw new Error(`timed out waiting for ${what}: ${JSON.stringify(value)}`);
    await delay(100);
  }
}

(async () => {
  const port = await reservePort();
  const browser = launch({ args: ["--url", page, "--allow-downloads"], debugPort: port });
  const { call } = browser;
  let failed = false;
  try {
    const health = await call("api.health");
    assert.equal(health.status, "ready");
    assert.equal(health.shell, true);
    assert.equal(health.content, true);
    assert.match(health.instanceId, /^i-[0-9a-f]{32}$/);
    assert.equal(health.endpointRecord, true);
    assert.equal(health.permissionPolicy, "default-deny");
    assert.equal(health.rendererCrashReports, 0);
    assert.equal(health.profile.ephemeral, true);
    assert.equal(health.pipe, `\\\\.\\pipe\\agentgo-${health.instanceId}`);

    // Typed errors.
    await rejects(call("made.up"), "method.notFound");
    await rejects(call("window.new"), "capability.unsupported");
    await rejects(call("tab.navigate", { url: "javascript:alert(1)" }), "invalid.params");
    await rejects(call("tabs.status", { tabId: "content-99" }), "target.stale");
    const badEnvelope = await browser.raw("api.health", {});
    assert.equal(badEnvelope.v, 1);

    const version = await call("api.version");
    assert.equal(version.protocol, "0.1.0");
    assert.match(version.chromium, /^\d+\./);

    // Capabilities come from the one registry: nothing implemented is also
    // listed as planned, and spot-checked names are present.
    const capabilities = await call("api.capabilities");
    for (const name of ["tabs.navigate", "observe.page", "act.click", "workflow.batch", "download.start", "browser.crash_report"]) {
      assert(capabilities.implemented.includes(name), name);
    }
    assert(capabilities.planned.includes("window.new"));
    assert.equal(capabilities.implemented.filter((name) => capabilities.planned.includes(name)).length, 0);
    assert.equal((await call("browser.crash_report")).count, 0);

    // Profiles.
    assert((await call("profile.list")).profiles.some((profile) => profile.id === "default"));
    assert.equal((await call("profile.create", { id: "smoke-profile", name: "Smoke Profile", ephemeral: true })).profile.id, "smoke-profile");
    await rejects(call("profile.delete", { id: "smoke-profile", confirm: true }), "policy.denied"); // operator only
    await rejects(operator(port, "profile.delete", { id: "smoke-profile" }), "approval.required");
    assert.equal((await operator(port, "profile.delete", { id: "smoke-profile", confirm: true })).deleted, true);

    // Search resolution is Google-only.
    assert.match((await call("navigate.search", { input: "agentgo browser" })).url, /^https:\/\/www\.google\.com\/search\?q=agentgo%20browser$/);
    assert.equal((await call("search.resolve", { input: "example.com" })).url, "https://example.com");

    // Settings: an agent changes only how the browser looks; everything else
    // is the person's. Secret-shaped keys are refused even for the person.
    assert.equal((await call("settings.get")).searchEngine, "Google");
    assert.equal((await call("settings.set", { settings: { motion: false } })).motion, false);
    assert.equal((await call("settings.get")).motion, false);
    for (const key of ["smokeMarker", "agentBlockedDomains", "agentAnimations", "shortcuts", "syncFolder"]) {
      await rejects(call("settings.set", { settings: { [key]: null } }), "policy.denied");
    }
    await rejects(operator(port, "settings.set", { settings: { apiToken: "x" } }), "invalid.params");
    assert.equal((await operator(port, "settings.set", { settings: { offerToSavePasswords: false } })).offerToSavePasswords, false);
    await operator(port, "settings.set", { settings: { offerToSavePasswords: true } });

    // Tabs. The start page loaded with its real title.
    const status = await until(() => call("tabs.status"), (s) => !s.loading && s.title === "AgentGo fixture — interaction page", "start page");
    assert.equal(status.url, page);
    assert.equal((await call("tabs.navigate", { url: page })).accepted, true);
    const created = await call("tabs.create", { url: landing });
    assert.equal(created.tabId, "content-2");
    const two = await until(() => call("tabs.list"), (t) => t.tabs.length === 2 && !t.tabs.some((tab) => tab.loading), "second tab");
    assert.equal(two.activeTabId, "content-2");
    assert.equal(two.tabs.find((tab) => tab.tabId === "content-2").url, landing);
    assert.equal((await call("tabs.activate", { tabId: "content-1" })).activeTabId, "content-1");
    assert.equal((await call("tabs.close", { tabId: "content-2" })).activeTabId, "content-1");
    await rejects(call("tabs.close", { tabId: "content-1" }), "invalid.params"); // the last tab is protected
    assert.equal((await call("tabs.list")).tabs.length, 1);

    // Raw pointer and keyboard input.
    assert.equal((await call("pointer.move", { x: 120, y: 120 })).accepted, true);
    assert.equal((await call("pointer.click", { x: 120, y: 120 })).accepted, true);
    assert.equal((await call("pointer.scroll", { x: 120, y: 120, deltaY: 240 })).accepted, true);
    assert.equal((await call("keyboard.type", { text: "smoke" })).typed, 5);
    assert.equal((await call("keyboard.press", { key: "ENTER" })).key, "ENTER");
    await rejects(call("pointer.move", { x: -1, y: 0 }), "invalid.params");
    await rejects(call("keyboard.press", { key: "NOT_A_KEY" }), "invalid.params");

    // Site info and history with real titles; clearing is operator only.
    assert.equal((await call("siteinfo.get")).scheme, "file");
    const history = await call("history.list", { limit: 10 });
    assert.equal(history.items.find((item) => item.url === page).title, "AgentGo fixture — interaction page");
    await rejects(call("history.clear", { confirm: true }), "policy.denied");
    await rejects(operator(port, "history.clear"), "approval.required");
    await operator(port, "history.clear", { confirm: true });
    assert.equal((await call("history.list")).items.length, 0);

    // Downloads (enabled for this run by the operator's launch flag).
    const started = await call("download.start", { url: downloadPage });
    assert.equal(started.accepted, true);
    await until(() => call("download.progress", { id: started.id }), (p) => p.item.complete, "the explicit download");
    await call("tabs.navigate", { url: downloadPage }); // the page triggers its own download too
    const downloads = await until(
      () => call("downloads.list"),
      (list) => list.items.some((item) => item.suggestedName === "agentgo-smoke.txt" && item.complete),
      "the fixture download to complete",
    );
    const file = downloads.items.find((item) => item.suggestedName === "agentgo-smoke.txt");
    assert.equal(fs.readFileSync(file.path, "utf8"), "AgentGo download smoke");
    assert(file.path.startsWith(browser.root), "downloads stay inside the scoped directory");
    assert(Array.isArray((await call("download.progress")).items));
    for (const method of ["download.pause", "download.resume", "download.cancel"]) {
      await rejects(call(method, { id: 4294967295 }), "target.stale");
    }
    await rejects(call("downloads.clear"), "approval.required");
    await call("downloads.clear", { confirm: true });
    assert.equal((await call("download.list")).items.length, 0);

    for (const method of ["tab.reload", "tab.stop", "tab.back", "tab.forward"]) {
      assert.equal((await call(method)).tabId, "content-1", method);
    }

    // Window controls, including a 4K surface.
    assert.equal(typeof (await call("window.minimize")).minimized, "boolean");
    assert.equal(typeof (await call("window.maximize")).maximized, "boolean");
    assert.equal((await call("window.maximize")).maximized, false);
    assert.equal((await call("window.resize", { width: 3840, height: 2160 })).width, 3840);
    assert.equal((await call("pointer.move", { x: 1919, y: 1079 })).accepted, true);
    assert.equal((await call("window.resize", { width: 1200, height: 800 })).height, 800);
    await rejects(call("window.resize", { width: 100, height: 100 }), "invalid.params");

    // Durable instance record: stopped, no secrets, token removed at exit.
    const instanceDir = path.join(browser.root, "instances", health.instanceId);
    assert.match(fs.readFileSync(path.join(instanceDir, "token"), "utf8"), /^[0-9a-f]{64}$/);
    await call("browser.shutdown");
    assert.equal(await browser.exited, 0);
    const record = JSON.parse(fs.readFileSync(path.join(instanceDir, "endpoint.json"), "utf8"));
    const body = record.data ?? record;
    assert.equal(body.status, "stopped");
    assert.equal(body.exit_code ?? body.exitCode, 0);
    assert(!JSON.stringify(record).match(/token|secret|cookie/i));
    assert(!fs.existsSync(path.join(instanceDir, "token")), "the token does not outlive the instance");
    console.log(JSON.stringify({ ok: true, instance: health.instanceId, methods: capabilities.implemented.length }));
  } catch (error) {
    console.error(error);
    failed = true;
  } finally {
    await browser.close();
    process.exitCode = failed ? 1 : 0;
  }
})();
