// Milestone 3 and 7 gates: authenticated named pipe, per-session tab leases,
// two isolated instances, events, operator pause/stop, approvals bound to the
// exact action, and CAPTCHA handoff that the agent can never resolve itself.
const assert = require("node:assert/strict");
const { launch, attach, operator, cdpEval, reservePort, fixture, rejects, refOf, delay } = require("./harness");

(async () => {
  const port = await reservePort();
  // Agent A launches the browser over stdio; the test plays the operator
  // through the trusted chrome document.
  const one = launch({ args: ["--url", "about:blank"], debugPort: port });
  const two = launch({ args: ["--url", "about:blank"] });
  try {
    const infoA = await one.call("session.info");
    const infoTwo = await two.call("session.info");
    assert.equal(infoA.session, "stdio");
    assert.notEqual(infoA.instanceId, infoTwo.instanceId, "each launch is its own instance");
    assert.equal(infoA.profile.ephemeral, true, "agent launches without --profile are throwaway");
    assert.equal(infoA.controlMode, "approval");

    // --- authentication -------------------------------------------------
    const bad = await attach(one.root, infoA.instanceId, { token: "0".repeat(64) });
    assert.equal(bad.hello.ok, false);
    assert.equal(bad.hello.error.code, "unauthorized");
    await bad.closed; // the server drops unauthenticated clients
    // A token from another instance does not work either.
    const cross = await attach(one.root, infoA.instanceId, {
      token: require("node:fs").readFileSync(`${two.root}/instances/${infoTwo.instanceId}/token`, "utf8"),
    });
    assert.equal(cross.hello.error.code, "unauthorized");
    await cross.closed;

    const b = await attach(one.root, infoA.instanceId, { client: "agent-b" });
    assert.equal(b.hello.ok, true);
    assert.equal(b.hello.result.instanceId, infoA.instanceId);
    const sessionB = b.hello.result.session;
    assert.notEqual(sessionB, "stdio");
    await b.call("events.subscribe");

    // --- leases: sessions cannot touch each other's tabs ------------------
    await one.call("navigate.url", { url: fixture("agent.html") }); // A leases content-1
    const tabB = (await b.call("tabs.create", { url: fixture("challenge.html") })).tabId;
    await rejects(b.call("observe.page", { tabId: "content-1" }), "unauthorized");
    await rejects(b.call("tabs.navigate", { tabId: "content-1", url: "about:blank" }), "unauthorized");
    await rejects(b.call("tabs.close", { tabId: "content-1" }), "unauthorized");
    await rejects(one.call("act.click", { tabId: tabB, x: 5, y: 5 }), "unauthorized");
    assert.deepEqual((await b.call("tabs.list")).tabs.map((t) => t.tabId), [tabB]);
    assert.deepEqual((await one.call("tabs.list")).tabs.map((t) => t.tabId), ["content-1"]);
    // An attached agent cannot close a browser it did not launch... (stdio
    // launched this one, so only the pipe session is refused)
    const status = await one.call("agent.status");
    assert.equal(status.sessions.length, 2);
    assert(status.sessions.some((s) => s.client === "agent-b" && s.transport === "pipe" && s.tabs.includes(tabB)));

    // --- CAPTCHA handoff ---------------------------------------------------
    await b.call("tabs.activate", { tabId: tabB });
    await b.call("wait.load", { tabId: tabB });
    const challenged = await b.call("observe.page", { tabId: tabB });
    assert.equal(challenged.signals.captcha, true);
    assert.equal(challenged.handoff.reason, "captcha");
    const blocked = await rejects(b.call("act.click", { tabId: tabB, ref: refOf(challenged.page, "Continue") }), "handoff.required");
    assert.equal(blocked.data.tabId, tabB);
    await rejects(b.call("handoff.resolve", { tabId: tabB }), "policy.denied"); // agents cannot resume themselves
    assert.equal((await b.call("handoff.wait", { tabId: tabB, timeout: 300 })).resolved, false);
    // The operator cannot resume while the challenge is still there.
    await rejects(operator(port, "handoff.resolve", { tabId: tabB }), "handoff.required");
    assert.match(one.stderr(), /operator-notification "AgentGo needs you: human verification/);
    // The human completes it in the same tab, then resumes.
    const waiting = b.call("handoff.wait", { tabId: tabB, timeout: 20_000 });
    await cdpEval(port, "challenge.html", "document.getElementById('human').click(); true");
    await operator(port, "handoff.resolve", { tabId: tabB });
    assert.equal((await waiting).resolved, true);
    const resumed = await b.call("observe.page", { tabId: tabB });
    assert.equal(resumed.handoff, undefined);
    await b.call("act.click", { tabId: tabB, ref: refOf(resumed.page, "Continue") });
    assert.match((await b.call("observe.text", { tabId: tabB })).text, /verified by human\ncontinued/);

    // --- approvals: exact, single use, operator only ----------------------
    const pageA = (await one.call("observe.page")).page;
    const buy = refOf(pageA, "Buy now");
    const needed = await rejects(one.call("act.click", { ref: buy }), "approval.required");
    const approvalId = needed.data.approvalId;
    // Another session cannot use or see it.
    await rejects(b.call("approval.wait", { approvalId }), "invalid.params");
    const decision = one.call("approval.wait", { approvalId, timeout: 20_000 });
    const pendingView = await operator(port, "agent.status");
    assert.equal(pendingView.approvals[0].approvalId, approvalId);
    assert.match(pendingView.approvals[0].reason, /payment or purchase/);
    await operator(port, "agent.approval.respond", { approvalId, approve: true });
    assert.equal((await decision).status, "granted");
    // The approval is bound to its target: it cannot be spent on another one.
    await rejects(one.call("act.click", { ref: refOf(pageA, "Send profile"), approvalId }), "approval.required");
    await one.call("act.click", { ref: buy, approvalId });
    assert.match((await one.call("observe.text")).text, /PURCHASED/);
    // Single use: a second purchase needs a fresh approval.
    const again = await rejects(one.call("act.click", { ref: buy, approvalId }), "approval.required");
    assert.notEqual(again.data.approvalId, approvalId);
    // A denial is final for that request.
    await operator(port, "agent.approval.respond", { approvalId: again.data.approvalId, approve: false });
    await rejects(one.call("act.click", { ref: buy, approvalId: again.data.approvalId }), "policy.denied");
    assert.equal((await one.call("observe.text")).text.match(/PURCHASED/g).length, 1);

    // --- pause preserves, stop revokes -------------------------------------
    await operator(port, "agent.pause");
    let finished = false;
    const held = one.call("act.click", { ref: refOf(pageA, "Count clicks") }).then((r) => { finished = true; return r; });
    await delay(700);
    assert.equal(finished, false, "actions wait while paused");
    assert.equal((await one.call("observe.page")).title, "AgentGo action fixture", "reading still works while paused");
    await operator(port, "agent.resume");
    await held;
    assert.match((await one.call("observe.text")).text, /counter 1 trusted=true/);

    await operator(port, "agent.stop");
    await rejects(one.call("act.click", { ref: refOf(pageA, "Count clicks") }), "unauthorized");
    await rejects(b.call("tabs.create", { url: "about:blank" }), "unauthorized");
    await rejects(one.call("agent.resume"), "unauthorized"); // cannot un-stop itself
    await operator(port, "agent.resume");
    await one.call("act.click", { ref: refOf(pageA, "Count clicks") });

    // --- events carry increasing sequence numbers --------------------------
    const names = b.events.map((e) => e.event);
    for (const expected of ["handoff.requested", "handoff.resolved", "approval.requested", "approval.decided", "agent.control", "tab.navigated"]) {
      assert(names.includes(expected), `missing event ${expected} in ${names}`);
    }
    const seqs = b.events.map((e) => e.seq);
    assert.deepEqual(seqs, [...seqs].sort((x, y) => x - y));
    assert.equal(one.events.length, 0, "unsubscribed sessions receive no events");

    // --- disconnect releases leases, tabs stay for the human ---------------
    b.end();
    await b.closed;
    await delay(300);
    const after = await one.call("agent.status");
    assert.equal(after.sessions.length, 1);
    assert((await one.call("tabs.list")).tabs.some((t) => t.tabId === tabB), "the released tab is visible again");

    // The second instance was untouched throughout.
    assert.equal((await two.call("agent.status")).sessions.length, 1);
    assert.equal((await two.call("tabs.list")).tabs.length, 1);

    // --- presence: the toolbar shows connected agents, steadily ---------------
    const present = async () => (await one.call("agent.status")).agents.map((a) => a.agent);
    const before = await present();
    // One-shot command-line calls do not flash "agent connected".
    const cli = await attach(one.root, infoA.instanceId, { client: "agentgo-cli" });
    assert.deepEqual(await present(), before);
    // An agent using one-shot calls says when it starts and when it is done.
    assert.equal((await cli.call("agent.connect", { name: "Claude" })).connected, true);
    cli.end();
    await cli.closed;
    await delay(200);
    assert((await present()).includes("Claude"), "an explicit connection outlives the one-shot call");
    const label = await cdpEval(port, "chrome.html", "document.querySelector('#agentText').textContent");
    assert.match(label, /connected/);
    assert.equal((await one.call("agent.disconnect", { name: "Claude" })).wasConnected, true);
    assert(!(await present()).includes("Claude"));
    // A long-lived connection is present exactly while it is open.
    const live = await attach(one.root, infoA.instanceId, { client: "long-lived-agent" });
    assert((await present()).includes("long-lived-agent"));
    live.end();
    await live.closed;
    await delay(300);
    assert(!(await present()).includes("long-lived-agent"));

    console.log(JSON.stringify({ ok: true, events: names.length }));
  } catch (error) {
    console.error(error); // report the real failure before cleanup
    process.exitCode = 1;
  } finally {
    const codes = [await one.close(), await two.close()];
    assert.deepEqual(codes, [0, 0], `exit codes ${codes}\n${one.stderr().slice(-1500)}`);
  }
})().catch((error) => { console.error(error); process.exit(1); });
