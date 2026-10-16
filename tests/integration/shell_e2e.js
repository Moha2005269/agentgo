// The AgentGo window's own panels (shell.*), local skills (skill.*) and the
// recording checks (workflow.diff, verify, approve). Each shell command opens
// the matching panel for the person; the test reads the panel's document.
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { launch, operator, cdpEval, reservePort, fixture, rejects, delay } = require("./harness");

const page = fixture("agent.html");
const click = { method: "act.click", params: { find: { by: "css", selector: "#counter" } } };

async function panel(port, expression) {
  for (let i = 0; i < 40; i += 1) {
    const value = await cdpEval(port, "tools.html", expression).catch(() => null);
    if (value) return value;
    await delay(150);
  }
  const seen = await cdpEval(port, "tools.html", "document.body.className + ' open=' + document.body.dataset.open + ' ' + [...document.querySelectorAll('[data-section]')].map(e => e.dataset.section).join(',')").catch((e) => e.message);
  throw new Error(`the panel never showed: ${expression} (panel: ${seen})`);
}

(async () => {
  const port = await reservePort();
  const browser = launch({ args: ["--url", "about:blank"], debugPort: port });
  const op = (method, params) => operator(port, method, params);
  try {
    await browser.call("navigate.url", { url: page });

    // A task, shown in the Agents panel.
    const task = await browser.call("agent.run", { goal: "count and fail", steps: [click, { method: "wait.text", params: { text: "never", timeout: 200 } }, click] });
    assert.equal(task.status, "failed");
    assert.equal((await browser.call("shell.agent_goal")).goal, "count and fail");
    await panel(port, "document.querySelector('.task-goal')?.textContent === 'count and fail'");
    assert.equal((await browser.call("shell.agent_step", { taskId: task.taskId })).step, 1);
    assert.equal((await browser.call("shell.agent_cost", { taskId: task.taskId })).used.steps, 2);
    const errors = await browser.call("shell.agent_error");
    assert.ok(errors.errors.some((e) => e.taskId === task.taskId && e.recovery.class === "timeout"));
    await panel(port, "document.querySelector('.step.failed') !== null");

    // The person's controls: edit the plan, ask for a replan, retry.
    await rejects(browser.call("shell.agent_edit_plan", { taskId: task.taskId, remove: [1] }), "policy.denied");
    const edited = await op("shell.agent_edit_plan", { taskId: task.taskId, remove: [1] });
    assert.deepEqual(edited.plan.map((s) => s.method), ["act.click", "act.click"]);
    const replan = await op("shell.agent_replan", { taskId: task.taskId, note: "try the other button" });
    assert.equal(replan.status, "replan");
    assert.equal((await browser.call("agent.monitor", { taskId: task.taskId })).note, "try the other button");
    await rejects(browser.call("shell.agent_retry", { taskId: task.taskId }), "policy.denied");
    assert.equal((await op("shell.agent_retry", { taskId: task.taskId })).retrying, true);
    for (let i = 0; i < 40 && (await browser.call("agent.monitor", { taskId: task.taskId })).status !== "done"; i += 1) await delay(150);
    assert.equal((await browser.call("agent.monitor", { taskId: task.taskId })).status, "done", "retried as its agent");

    // Handoff, artifacts, diff.
    assert.ok(Array.isArray((await browser.call("shell.agent_handoff")).handoffs));
    await browser.call("artifact.store", { text: "evidence" });
    const artifacts = await browser.call("shell.agent_artifacts");
    assert.ok(artifacts.artifacts.length >= 1);
    await panel(port, "document.querySelector('[data-section=artifacts]') !== null");
    assert.equal(typeof (await browser.call("shell.diff_view")).changes, "string");
    await panel(port, "document.querySelector('.diff') !== null");
    await rejects(browser.call("shell.agent_return_control"), "policy.denied");

    // Settings sections and other panels.
    assert.equal((await browser.call("shell.permission_center")).default, "refuse");
    await panel(port, "document.querySelector('[data-section=privacy]') !== null");
    assert.ok("controlMode" in (await browser.call("shell.policy_editor")));
    assert.equal(typeof (await browser.call("shell.settings", { section: "about" })), "object");
    assert.equal((await browser.call("shell.sidebar", { panel: "history" })).open, "history");
    await panel(port, "document.body.className === 'history'");
    await rejects(browser.call("shell.sidebar", { panel: "nowhere" }), "invalid.params");
    assert.ok((await browser.call("shell.quick_actions")).actions.length >= 5);
    assert.equal((await browser.call("shell.command_bar", { text: "typed" })).focused, true);
    assert.equal(await cdpEval(port, "chrome.html", "document.querySelector('#address').value"), "typed");

    // On the page: point at an element and preview an action.
    assert.equal((await browser.call("shell.target_highlight", { find: { by: "css", selector: "#counter" } })).shown, true);
    const preview = await browser.call("shell.action_preview", { method: "act.click", params: { ref: 1 } });
    assert.equal(preview.preview.risk, "act");
    const stored = await browser.call("artifact.store", { text: "<b>shown</b>", extension: "html" });
    const opened = await browser.call("shell.artifact_preview", { path: stored.path });
    assert.match(opened.url, /^file:\/\/\//);

    // Local skills.
    const name = `count-${process.pid}`;
    await rejects(browser.call("skill.install_local", { name, steps: [{ method: "act.fill", params: { text: "{{who}}" } }] }), "invalid.params");
    const installed = await browser.call("skill.install_local", {
      name, description: "count clicks", params: { times: "1" },
      steps: [click, { method: "wait.text", params: { text: "counter {{times}}", timeout: 3000 } }],
      expect: { text: "counter {{times}}" },
    });
    assert.equal(installed.version, "0.1.0");
    assert.equal((await browser.call("skill.validate", { name })).valid, true);
    assert.equal((await browser.call("skill.validate", { skill: { steps: [{ method: "observe.text" }], params: { x: 1 } } })).valid, false);
    assert.equal((await browser.call("skill.example", { name })).call.method, "skill.test");
    assert.deepEqual((await browser.call("skill.permission_requirements", { name })).acting, ["act.click"]);
    await browser.call("navigate.url", { url: page });
    const tested = await browser.call("skill.test", { name, params: { times: "1" } });
    assert.equal(tested.passed, true, JSON.stringify(tested));
    assert.equal((await browser.call("skill.update", { name, description: "counts" })).version, "0.1.1");
    await browser.call("skill.disable", { name });
    await rejects(browser.call("skill.test", { name }), "policy.denied");
    await browser.call("skill.enable", { name });
    assert.equal((await browser.call("skill.domain_adapter", { name, domain: "example.com" })).domain, "example.com");
    assert.ok((await browser.call("skill.domain_adapter", { domain: "shop.example.com" })).skills.some((s) => s.name === name));
    const browserList = await browser.call("shell.skill_browser");
    assert.ok(browserList.skills.some((s) => s.name === name));
    await panel(port, `document.querySelector('[data-skill-toggle="${name}"]') !== null`);

    // A recording becomes a skill; recordings are compared, verified, approved.
    const rec = `rec-${process.pid}`;
    await browser.call("navigate.url", { url: page });
    await browser.call("workflow.record", { name: rec });
    const outline = (await browser.call("observe.page")).page;
    const ref = outline.split("\n").find((l) => l.includes("Count clicks")).match(/\[(\d+)\]/)[1];
    await browser.call("act.click", { ref: Number(ref) });
    await browser.call("workflow.record", { stop: true });
    const compiled = await browser.call("skill.compile", { workflow: rec, name: `${rec}-skill` });
    assert.equal(compiled.steps[0].params.find.name, "Count clicks");
    assert.equal(compiled.steps[0].params.ref, undefined);
    const parameterized = await browser.call("skill.parameterize", { name: `${rec}-skill`, values: { "Count clicks": "label" } });
    assert.equal(parameterized.params.label, "Count clicks");
    assert.equal((await browser.call("skill.test", { name: `${rec}-skill` })).passed, true);
    assert.equal((await browser.call("workflow.diff", { a: rec, b: rec })).same, true);
    const verified = await browser.call("workflow.verify", { name: rec });
    assert.equal(verified.broken, 0);
    assert.equal(verified.steps[0].onPage, true);
    assert.equal(verified.approved, false);
    await rejects(browser.call("workflow.approve", { name: rec }), "policy.denied");
    await op("workflow.approve", { name: rec });
    assert.equal((await browser.call("workflow.verify", { name: rec })).approved, true);
    assert.ok((await browser.call("shell.workflow_editor")).workflows.some((w) => w.name === rec && w.approved));
    await browser.call("skill.remove_local", { name });
    await browser.call("skill.remove_local", { name: `${rec}-skill` });
    await rejects(browser.call("skill.example", { name }), "invalid.params");
    console.log("shell_e2e: PASS");
  } finally {
    await browser.close();
  }
})().catch((error) => {
  console.error(error);
  process.exit(1);
});
