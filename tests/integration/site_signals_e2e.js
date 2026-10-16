// Real-world page behavior that earlier runs got wrong: a tiny page keeps its
// heading, a Cloudflare-style full-page check is recognised (and waited out
// when it clears itself), and an ordinary Cloudflare site is not mistaken for
// one. AgentGo only detects a check; it never tries to solve it.
const assert = require("node:assert/strict");
const { launch, serve } = require("./harness");

(async () => {
  const site = await serve();
  const browser = launch({ args: ["--url", "about:blank"] });
  try {
    // example.com-shaped page: the heading must survive in the outline.
    const minimal = await browser.call("navigate.url", { url: site.url("minimal.html") });
    assert.match(minimal.page, /^# Example Domain$/m, minimal.page);
    assert.match(minimal.page, /\[\d+\] link "Learn more"/);
    assert.equal(minimal.signals, undefined);

    // A site that merely loads Cloudflare's analytics script is not a check.
    const analytics = await browser.call("navigate.url", { url: site.url("cf_analytics.html") });
    assert.match(analytics.page, /^# Documentation$/m);
    assert.equal(analytics.signals, undefined, "analytics script is not a challenge");
    assert.equal(analytics.handoff, undefined);

    // A check that finishes by itself is waited out: no handoff, real page.
    const started = Date.now();
    const cleared = await browser.call("navigate.url", { url: site.url("cf_interstitial.html?pass=1500") });
    assert.match(cleared.page, /^# Welcome through$/m, cleared.page);
    assert.equal(cleared.handoff, undefined);
    assert.equal(cleared.signals, undefined);
    assert(Date.now() - started < 7_000, "a short check must not cost the full grace period");

    // A check that never ends starts a human handoff after the grace period.
    const stuck = await browser.call("navigate.url", { url: site.url("cf_interstitial.html") });
    assert.equal(stuck.signals.captcha, true);
    assert.equal(stuck.signals.challenge, "interstitial");
    assert.equal(stuck.handoff.reason, "captcha");
    assert(stuck.ms >= 7_000, `the grace period was skipped (${stuck.ms} ms)`);
    // The agent may give up on the page by going elsewhere; nothing else.
    assert.equal((await browser.call("observe.page")).handoff.reason, "captcha");
    const away = await browser.call("navigate.url", { url: site.url("fixture.html") });
    assert.equal(away.handoff, undefined);
    assert.equal((await browser.call("agent.status")).handoffs.length, 0);

    console.log("site signals e2e: ok");
  } finally {
    site.close();
    await browser.close();
  }
})().catch((error) => {
  console.error(error);
  process.exit(1);
});
