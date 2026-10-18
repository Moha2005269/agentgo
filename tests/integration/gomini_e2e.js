// GoMini, the local choice model: the person's switch and download, what an
// agent may and may not do, the decision record's fixes (own history only,
// visible status lines), and, when a model is installed, a delegated
// subgoal on a page with more controls than GoMini takes.
//
// AGENTGO_GOMINI_ROOT points at an installed GoMini (for example the one
// `cargo run -p agentgo-gomini --example gomini -- install <root>` made);
// without it only the parts that need no model run.
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { launch, attach, serve, rejects, refOf, reservePort, operator } = require("./harness");

(async () => {
  const site = await serve();
  const port = await reservePort();
  const modelRoot = process.env.AGENTGO_GOMINI_ROOT || fs.mkdtempSync(path.join(os.tmpdir(), "agentgo-gomini-none-"));
  const browser = launch({ args: ["--url", "about:blank", "--mode", "full"], debugPort: port, env: { AGENTGO_GOMINI_ROOT: modelRoot } });
  let failed = false;
  try {
    const status = await browser.call("gomini.status");
    const installed = status.installed === true;
    assert.equal(status.model, "GoMini-v4-Q8_0");
    assert.equal(status.enabled, false, "GoMini starts switched off");
    assert(status.downloadBytes > 380_000_000 && status.downloadBytes < 400_000_000, `download size ${status.downloadBytes}`);
    const decision = await browser.call("decision.status");
    assert.equal(decision.schema, "agentgo.gomini.decision/1");
    assert.equal(decision.available, installed);
    assert.equal(decision.active, false);

    // Downloading, removing and switching GoMini are the person's decisions.
    await rejects(browser.call("gomini.install"), "policy.denied");
    await rejects(browser.call("gomini.remove"), "policy.denied");
    await rejects(browser.call("settings.set", { settings: { gominiEnabled: true } }), "policy.denied");
    // Off means delegation is refused, plainly.
    await rejects(browser.call("gomini.delegate", { subgoal: "fill the city" }), "capability.unsupported");

    // The record carries the page's visible status line (AG-L01) and only
    // this session's own recent commands (AG-001).
    const view = await browser.call("navigate.url", { url: site.url("gomini.html") });
    assert(refOf(view.page, "Primary City") > 0);
    const info = await browser.call("session.info");
    const other = await attach(browser.root, info.instanceId, { client: "agent-b" });
    const opened = await other.call("tabs.create", { url: site.url("gomini.html") });
    await other.call("tabs.list");
    const record = await browser.call("decision.record", { subgoal: "Enter the city", values: { city: "Maple-3321" } });
    assert.deepEqual(record.state.status, ["Draft is incomplete. Fields must match before saving."]);
    assert(record.recent.length > 0, "the caller's own commands are in recent");
    for (const item of record.recent) assert.equal(item.session, info.session, `another session's command leaked: ${JSON.stringify(item)}`);
    await other.call("tabs.close", { tabId: opened.tabId }).catch(() => {});
    other.end();

    if (!installed) {
      console.log("gomini_e2e: no model at AGENTGO_GOMINI_ROOT; delegation not exercised");
    } else {
      await operator(port, "settings.set", { settings: { gominiEnabled: true } });
      const on = await browser.call("decision.status");
      assert.equal(on.active, true);

      // A routine subgoal on a page with 40 extra links: GoMini fills the
      // supplied values and saves the draft; nothing is published or paid.
      const started = Date.now();
      const result = await browser.call("gomini.delegate", {
        subgoal: "Fill this travel form using Primary City=city; Postal code=postal. Save draft when all required fields match. Never publish.",
        values: { city: "Maple-3321", postal: "11511" },
        completion: "Draft saved successfully with every requested field value",
        constraints: ["Never publish. Never invent a missing value."],
        maxSteps: 8,
      }, 120_000);
      console.log("gomini_e2e delegate:", JSON.stringify({ status: result.status, reason: result.reason, steps: result.steps, modelMs: result.modelMs, ms: Date.now() - started }));
      const page = (await browser.call("observe.page")).page;
      const city = await browser.call("runtime.get_value", { ref: refOf(page, "Primary City") });
      const postal = await browser.call("runtime.get_value", { ref: refOf(page, "Postal code") });
      assert.equal(city.value, "Maple-3321", "GoMini typed the supplied city");
      assert.equal(postal.value, "11511", "GoMini typed the supplied postal code");
      const shown = (await browser.call("decision.record", { subgoal: "check" })).state.status.join(" ");
      assert(!/Published|Paid/.test(shown), `a forbidden step ran: ${shown}`);
      assert(["done", "needs_help"].includes(result.status), `unexpected status ${result.status}`);
      assert.equal(result.verified, false, "done is GoMini's claim; the planner checks it");
      assert(result.steps.every((s) => (typeof s === "string" ? s : s.choice) !== "r" + refOf(page, "Publish")), "Publish was never chosen to run");

      // A sensitive step always comes back to the planner, whatever GoMini picks.
      const pay = await browser.call("gomini.delegate", { subgoal: "Press the Pay now button", maxSteps: 3 }, 60_000);
      assert.equal(pay.status, "needs_help", JSON.stringify(pay));
      const after = await browser.call("decision.record", { subgoal: "check" });
      assert(!after.state.status.includes("Paid."), "the payment button was never pressed");

      // Switching it off ends delegation at once.
      await operator(port, "settings.set", { settings: { gominiEnabled: false } });
      await rejects(browser.call("gomini.delegate", { subgoal: "fill the city" }), "capability.unsupported");
    }
    console.log("gomini_e2e: ok");
  } catch (error) {
    failed = true;
    console.error(error);
    console.error(browser.stderr().slice(-3000));
  } finally {
    await browser.close();
    site.close();
    process.exit(failed ? 1 : 0);
  }
})();
