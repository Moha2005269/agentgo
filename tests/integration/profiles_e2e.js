// Side profiles: separate cookies and storage per profile in one window,
// in-memory profiles, locks, preferences, cloning, site reset and parallel
// lanes in their own profiles.
const assert = require("node:assert/strict");
const { launch, operator, reservePort, serve, rejects } = require("./harness");

(async () => {
  const site = await serve();
  const port = await reservePort();
  const browser = launch({ args: ["--url", "about:blank"], debugPort: port });
  const op = (method, params) => operator(port, method, params);
  const url = site.url("fixture.html");
  try {
    // A cookie in main is not seen by a side profile.
    const main = (await browser.call("navigate.url", { url })).tabId || (await browser.call("tab.status")).tabId;
    await browser.call("storage.cookie_set", { tabId: main, name: "who", value: "main" });
    const work = await browser.call("profile.open", { profile: "work", url });
    assert.equal(work.profile, "work");
    await browser.call("wait.load", { tabId: work.tabId });
    const workCookies = await browser.call("storage.cookies_list", { tabId: work.tabId });
    assert.ok(!workCookies.cookies.some((c) => c.name === "who"), "profiles do not share cookies");
    await browser.call("storage.cookie_set", { tabId: work.tabId, name: "who", value: "work" });
    const parts = await browser.call("profile.storage_partition");
    assert.ok(parts.profiles.some((p) => p.profile === "work" && p.open && p.tabs.includes(work.tabId)));
    assert.match((await browser.call("profile.storage_partition", { profile: "work" })).path, /side-work$/);

    // Selected profile: new tabs open there.
    await browser.call("profile.select", { profile: "work" });
    const second = await browser.call("tabs.create", { url });
    assert.equal(second.profile, "work");
    await browser.call("profile.select", { profile: "main" });
    assert.equal((await browser.call("tabs.create", { url: "about:blank" })).profile, undefined);

    // In memory: nothing shared, nothing kept.
    const temp = await browser.call("profile.incognito", { url });
    assert.match(temp.profile, /^incognito-/);
    assert.equal((await browser.call("profile.storage_partition", { profile: temp.profile })).path, "memory");
    const eph = await browser.call("profile.ephemeral", {});
    assert.match(eph.profile, /^ephemeral-/);

    // Preferences.
    assert.equal((await browser.call("profile.cookie_policy", { profile: "work", policy: "block_third_party" })).policy, "block_third_party");
    await rejects(browser.call("profile.cookie_policy", { profile: "work", policy: "bad" }), "invalid.params");
    await rejects(browser.call("profile.cookie_policy", { policy: "all" }), "policy.denied");
    assert.equal((await browser.call("profile.proxy", { profile: "work", mode: "direct" })).proxy.mode, "direct");
    assert.equal((await browser.call("profile.accessibility_preferences", { profile: "work", fontSize: 20 })).set.fontSize, 20);
    assert.equal((await browser.call("profile.permission_defaults", { profile: "work", kind: "popups", setting: "block" })).setting, "block");
    await rejects(browser.call("profile.permission_defaults", { profile: "work", kind: "camera", setting: "allow" }), "policy.denied");
    assert.equal((await browser.call("profile.autofill_policy", { profile: "work", enabled: false })).cards, false);
    assert.equal((await browser.call("profile.storage_partition", { profile: "work" })).preferences["webkit.webprefs.default_font_size"], 20);

    // Saved sign-ins are bound to main unless the person binds them.
    assert.equal((await browser.call("profile.password_vault_binding", { profile: "work" })).bound, false);
    await rejects(browser.call("credentials.fill", { tabId: work.tabId }), "policy.denied");
    await rejects(browser.call("profile.password_vault_binding", { profile: "work", bind: true }), "policy.denied");
    assert.equal((await op("profile.password_vault_binding", { profile: "work", bind: true })).bound, true);

    // Site reset in a side profile.
    const reset = await browser.call("profile.reset_site", { profile: "work", origin: url });
    assert.equal(reset.cleared, true);
    const after = await browser.call("storage.cookies_list", { tabId: work.tabId });
    assert.ok(!after.cookies.some((c) => c.name === "who"), "the site was reset");
    assert.ok((await browser.call("storage.cookies_list", { tabId: main })).cookies.some((c) => c.name === "who"), "main keeps its cookie");

    // Lock: the person keeps agents out.
    await rejects(browser.call("profile.lock", { profile: "work" }), "policy.denied");
    await op("profile.lock", { profile: "work" });
    await rejects(browser.call("observe.text", { tabId: work.tabId }), "policy.denied");
    await rejects(browser.call("profile.open", { profile: "work" }), "policy.denied");
    await op("profile.unlock", { profile: "work" });
    assert.ok((await browser.call("observe.text", { tabId: work.tabId })).text !== undefined);

    // Close and clone.
    await rejects(browser.call("profile.clone", { profile: "work", as: "work2" }), "invalid.params");
    const closed = await browser.call("profile.close", { profile: "work" });
    assert.equal(closed.closed, true);
    assert.equal(closed.tabs.length, 2);
    const cloned = await browser.call("profile.clone", { profile: "work", as: "work2" });
    assert.ok(cloned.files > 0);
    const reopened = await browser.call("profile.open", { profile: "work2", url });
    await browser.call("wait.load", { tabId: reopened.tabId });
    assert.equal((await browser.call("profile.storage_partition", { profile: "work2" })).preferences["webkit.webprefs.default_font_size"], 20, "preferences come along");

    // Parallel lanes, each in its own profile.
    const lanes = await browser.call("agent.parallel_profiles", { lanes: [
      { url, steps: [{ method: "storage.cookie_set", params: { name: "lane", value: "a" } }, { method: "storage.cookies_list", params: { name: "lane" } }] },
      { url, steps: [{ method: "storage.cookies_list", params: { name: "lane" } }] },
    ] });
    assert.equal(lanes.done, 2, JSON.stringify(lanes));
    const second_lane = lanes.lanes[1].results[0].result.cookies;
    assert.equal(second_lane.length, 0, "lanes do not share cookies");
    console.log("profiles_e2e: PASS");
  } finally {
    await browser.close();
    site.close();
  }
})().catch((error) => {
  console.error(error);
  process.exit(1);
});
