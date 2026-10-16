// The agent task engine: plans made by the calling model, stored and run by
// AgentGo with budgets, retries, checkpoints, queues, parallel tabs, batch
// transactions, recovery helpers, tab locks and resuming after a crash.
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { launch, attach, fixture, rejects, delay } = require("./harness");

const page = fixture("agent.html");
const click = { method: "act.click", params: { find: { by: "css", selector: "#counter" } } };
const counted = (n) => ({ method: "wait.text", params: { text: `counter ${n}`, timeout: 3000 } });

(async () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "agentgo-tasks-"));
  let browser = launch({ args: ["--url", "about:blank"], dataRoot: root });
  try {
    await browser.call("navigate.url", { url: page });

    // A plan is checked against the registry before anything runs.
    await rejects(browser.call("agent.plan", { steps: [{ method: "no.such" }] }), "method.notFound");
    await rejects(browser.call("agent.plan", { steps: [{ method: "agent.run" }] }), "invalid.params");
    const plan = await browser.call("agent.plan", { goal: "count twice", steps: [click, counted(1), click, counted(2)] });
    assert.equal(plan.status, "planned");
    assert.deepEqual(plan.plan.map((s) => s.readOnly), [false, true, false, true]);
    const preview = await browser.call("agent.preview", { taskId: plan.taskId });
    assert.equal(preview.acting, 2);
    assert.equal(preview.executed, false);
    assert.ok(preview.plan[0].description);
    assert.equal((await browser.call("agent.dry_run", { steps: [click] })).steps, 1);
    assert.equal((await browser.call("batch.preview", { taskId: plan.taskId })).steps, 4);

    // One step, then the rest; a checkpoint in between.
    const one = await browser.call("agent.act", { taskId: plan.taskId });
    assert.equal(one.status, "paused", JSON.stringify(one.error));
    assert.equal(one.cursor, 1);
    const cp = await browser.call("agent.checkpoint", { taskId: plan.taskId, name: "after-first" });
    assert.equal(cp.cursor, 1);
    const ran = await browser.call("agent.run", { taskId: plan.taskId });
    assert.equal(ran.status, "done", JSON.stringify(ran));
    assert.equal(ran.results.length, 4);
    const explained = await browser.call("agent.explain", { taskId: plan.taskId });
    assert.ok(explained.steps.every((s) => s.state === "done" && s.does));
    const monitor = await browser.call("agent.monitor");
    assert.ok(monitor.tasks.some((t) => t.taskId === plan.taskId && t.status === "done"));
    const restored = await browser.call("agent.restore", { taskId: plan.taskId, name: "after-first" });
    assert.equal(restored.cursor, 1);
    assert.equal((await browser.call("agent.monitor", { taskId: plan.taskId })).status, "planned");

    // A failing step stops the task; replan replaces the rest; retry runs on.
    const failing = await browser.call("agent.run", { goal: "fails", steps: [{ method: "wait.text", params: { text: "never there", timeout: 200 } }, click] });
    assert.equal(failing.status, "failed");
    assert.equal(failing.error.code, "timeout");
    assert.equal(failing.error.step, 0);
    const replanned = await browser.call("agent.replan", { taskId: failing.taskId, steps: [{ method: "observe.text" }] });
    assert.equal(replanned.plan.length, 1);
    assert.equal((await browser.call("agent.retry", { taskId: failing.taskId })).status, "done");

    // Retries: a step that times out is tried maxAttempts times.
    await browser.call("recovery.max_attempts", { attempts: 2 });
    const tried = await browser.call("agent.run", { steps: [{ method: "wait.text", params: { text: "never there", timeout: 150 } }] });
    assert.equal(tried.results[0].tries, 2);
    await browser.call("recovery.max_attempts", { attempts: 1 });

    // Budget: a task stops when its step budget is used.
    const budget = await browser.call("agent.plan", { steps: [{ method: "observe.text" }, { method: "observe.text" }, { method: "observe.text" }], budget: { maxSteps: 1 } });
    assert.equal((await browser.call("agent.run", { taskId: budget.taskId })).status, "budget");
    assert.equal((await browser.call("agent.budget", { taskId: budget.taskId, maxSteps: 10 })).budget.maxSteps, 10);
    assert.equal((await browser.call("agent.run", { taskId: budget.taskId })).status, "done");

    // Compare and merge results.
    const a = await browser.call("agent.run", { steps: [{ method: "observe.text" }] });
    const b = await browser.call("agent.run", { steps: [{ method: "observe.text" }] });
    assert.equal((await browser.call("agent.compare", { a: a.taskId, b: b.taskId })).equal, true);
    assert.equal((await browser.call("agent.compare", { a: [{ result: 1 }], b: [{ result: 2 }] })).equal, false);
    const merged = await browser.call("agent.merge_results", { lists: [[{ id: 1 }, { id: 2 }], [{ id: 2 }, { id: 3 }]], key: "id", dedupe: true });
    assert.equal(merged.count, 3);
    assert.equal((await browser.call("agent.deduplicate_results", { items: [{ x: 1 }, { x: 1 }, { x: 2 }] })).count, 2);

    // Queue and priority: the higher priority runs first.
    const low = await browser.call("agent.plan", { goal: "low", steps: [{ method: "observe.text" }], priority: 1 });
    const high = await browser.call("agent.plan", { goal: "high", steps: [{ method: "observe.text" }] });
    await browser.call("agent.priority", { taskId: high.taskId, priority: 5 });
    await browser.call("agent.queue", { taskId: low.taskId });
    await browser.call("agent.queue", { taskId: high.taskId });
    assert.deepEqual((await browser.call("agent.queue")).queue.map((t) => t.goal), ["high", "low"]);
    const queued = await browser.call("agent.queue", { run: true });
    assert.deepEqual(queued.ran.map((t) => t.taskId), [high.taskId, low.taskId]);

    // Subtask under a parent.
    const sub = await browser.call("agent.subtask", { taskId: plan.taskId, goal: "child", steps: [{ method: "observe.text" }] });
    assert.equal(sub.parent, plan.taskId);
    assert.equal(sub.status, "done");

    // Parallel lanes on two new tabs.
    const lanes = await browser.call("agent.parallel_tabs", { lanes: [
      { url: page, steps: [click, counted(1)] },
      { url: page, steps: [click, click, counted(2)] },
    ] });
    assert.equal(lanes.done, 2, JSON.stringify(lanes));

    // Batch transaction with UI rollback.
    const batch = await browser.call("batch.begin", { name: "nav" });
    await browser.call("batch.add", { batchId: batch.batchId, method: "navigate.url", params: { url: fixture("fixture.html") } });
    await browser.call("batch.add", { batchId: batch.batchId, steps: [{ method: "observe.text" }, { method: "observe.text" }] });
    assert.equal((await browser.call("batch.dedupe", { batchId: batch.batchId })).removed, 1);
    const key = await browser.call("batch.idempotency_key", { batchId: batch.batchId });
    assert.match(key.idempotencyKey, /^batch-[0-9a-f]{16}$/);
    const committed = await browser.call("agent.run", { taskId: batch.batchId, idempotencyKey: key.idempotencyKey });
    assert.equal(committed.status, "done");
    assert.equal((await browser.call("agent.run", { taskId: batch.batchId, idempotencyKey: key.idempotencyKey })).status, "done", "replayed, not rerun");
    assert.equal((await browser.call("batch.transaction_status", { batchId: batch.batchId })).status, "done");
    assert.match((await browser.call("tab.status")).url, /fixture\.html/);
    const rolled = await browser.call("batch.rollback_ui", { batchId: batch.batchId });
    assert.ok(rolled.tabs.some((t) => t.restored));
    assert.match((await browser.call("tab.status")).url, /agent\.html/);
    await browser.call("batch.replan", { batchId: batch.batchId, steps: [{ method: "observe.text" }] });
    assert.ok((await browser.call("batch.wait_until", { text: "Count clicks", timeout: 2000 })).met);

    // Recovery helpers.
    assert.equal((await browser.call("recovery.classify_error", { code: "target.stale" })).class, "stale_reference");
    assert.ok(Array.isArray((await browser.call("recovery.popup_blocked")).blocked));
    assert.ok(Array.isArray((await browser.call("recovery.permission_denied")).denied));
    assert.equal((await browser.call("recovery.false_completion", { text: "Count clicks" })).verified, true);
    assert.equal((await browser.call("recovery.false_completion", { text: "no such words", timeout: 300 })).verified, false);

    // A tab lock keeps the tab for the same client across reconnects.
    const info = await browser.call("session.info");
    const first = await attach(root, info.instanceId, { client: "locker" });
    const own = await first.call("tabs.create", { url: "about:blank" });
    assert.equal((await first.call("agent.lock", { tabId: own.tabId })).locked, true);
    first.end();
    await first.closed;
    await delay(300);
    const other = await attach(root, info.instanceId, { client: "someone-else" });
    await rejects(other.call("tabs.status", { tabId: own.tabId }), "unauthorized");
    other.end();
    const again = await attach(root, info.instanceId, { client: "locker" });
    assert.ok((await again.call("tabs.status", { tabId: own.tabId })).url);
    assert.equal((await again.call("agent.unlock", { tabId: own.tabId })).locked, false);
    again.end();

    // A task that was running when the browser died can be resumed.
    const slow = await browser.call("agent.plan", { goal: "slow", steps: [{ method: "wait.text", params: { text: "never there", timeout: 20000 } }, { method: "observe.text" }] });
    browser.call("agent.run", { taskId: slow.taskId }).catch(() => {});
    await delay(1500);
    browser.child.kill();
    await browser.exited;
    browser = launch({ args: ["--url", page], dataRoot: root });
    const crashed = await browser.call("agent.resume_after_crash");
    assert.ok(crashed.interrupted.some((t) => t.taskId === slow.taskId), JSON.stringify(crashed));
    await browser.call("agent.replan", { taskId: slow.taskId, steps: [{ method: "observe.text" }] });
    assert.equal((await browser.call("agent.resume_after_crash", { taskId: slow.taskId })).status, "done");
    console.log("tasks_e2e: PASS");
  } finally {
    await browser.close();
    fs.rmSync(root, { recursive: true, force: true });
  }
})().catch((error) => {
  console.error(error);
  process.exit(1);
});
