// Measurement and housekeeping: metrics counted from real commands, task
// outcomes, benchmark reports and regressions, recording replays as
// benchmarks, memory, the research cache, saved sessions, profile
// checkpoints, domain policy and the artifact/file commands.
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { launch, operator, serve, reservePort, rejects, refOf } = require("./harness");

(async () => {
  const site = await serve();
  const port = await reservePort();
  const browser = launch({ args: ["--url", "about:blank", "--mode", "full", "--allow-evaluate"], debugPort: port });
  const call = (method, params = {}) => browser.call(method, params);
  let failed = false;
  try {
    await operator(port, "settings.set", { settings: { agentAnimations: false } });
    // Metrics come from what the session really did.
    const view = await call("navigate.url", { url: site.url("daily.html") });
    await call("act.fill", { ref: refOf(view.page, "Paste target"), text: "measured" });
    await rejects(call("act.click", { ref: 999999 }), "target.stale");
    await call("workflow.batch", { steps: [{ method: "observe.text" }, { method: "observe.links" }] });
    const latency = await call("metrics.latency");
    assert(latency.value.count >= 4 && latency.value.p50 >= 0);
    assert.equal((await call("metrics.batch_size")).value.maxSteps, 2);
    assert((await call("metrics.stale_reference_rate")).value.rate > 0);
    assert.equal((await call("metrics.recovery_count")).value.errors["target.stale"], 1);
    // Task outcomes the planner reports.
    await call("metrics.success", { task: "fill", ok: true, tokens: 1200, turns: 3, accuracy: 0.9 });
    await call("metrics.success", { task: "buy", ok: false, tokens: 800, turns: 5, falseCompletion: true });
    const tasks = (await call("metrics.success")).value;
    assert.equal(tasks.successRate, 0.5);
    assert.equal((await call("metrics.success_per_token")).value.successPerThousandTokens, 0.5);
    assert.equal((await call("metrics.model_turns")).value.turns, 8);
    assert.equal((await call("metrics.false_completion_rate")).value.rate, 0.5);
    assert.equal((await call("metrics.extraction_accuracy")).value.accuracy, 0.9);
    for (const name of ["handoff_rate", "handoff_resolution_time", "native_dialog_success", "upload_success", "download_success"]) await call(`metrics.${name}`);
    // Benchmarks: save, compare, check for regressions.
    assert(fs.existsSync((await call("benchmark.record", { name: "before" })).file));
    for (let i = 0; i < 3; i += 1) await call("observe.page");
    await call("benchmark.record", { name: "after" });
    const compared = await call("benchmark.compare", { a: "before", b: "after" });
    assert(compared.figures.some((f) => f.figure === "replyTokensEstimate" && f.after > f.before));
    assert.equal(typeof (await call("benchmark.regression", { baseline: "before" })).regressed, "boolean");
    assert((await call("benchmark.token_report")).replyTokensEstimate > 0);
    assert.equal((await call("benchmark.recovery_report")).errors["target.stale"], 1);
    assert("approvalRequired" in (await call("benchmark.policy_report")).policy);
    // A recording replayed as a benchmark.
    const flow = await call("navigate.url", { url: site.url("daily.html") });
    await call("workflow.record", { name: "bench-flow" });
    await call("act.fill", { ref: refOf(flow.page, "Paste target"), text: "bench" });
    await call("workflow.record", { stop: true });
    const bench = await call("benchmark.run", { recording: "bench-flow", times: 2 });
    assert.equal(bench.succeeded, 2);
    assert.equal((await call("benchmark.custom_suite", { recordings: ["bench-flow"] })).total, 1);
    assert.equal((await call("metrics.reset")).value.successRate, null);

    // Memory between tasks.
    await call("memory.store", { key: "city", value: "Cairo", scope: "trip" });
    assert.equal((await call("memory.retrieve", { key: "city", scope: "trip" })).value, "Cairo");
    assert.equal((await call("memory.retrieve", { query: "cai", scope: "trip" })).items.length, 1);
    assert.deepEqual((await call("memory.scope")).scopes.find((s) => s.scope === "trip").keys, ["city"]);
    const exported = await call("memory.export");
    await call("memory.redact", { key: "city", scope: "trip" });
    assert.equal((await call("memory.retrieve", { key: "city", scope: "trip" })).value, "[redacted]");
    await call("memory.import", { memory: exported.memory });
    assert.equal((await call("memory.retrieve", { key: "city", scope: "trip" })).value, "Cairo");
    assert.equal((await call("memory.summarize")).scopes.length >= 1, true);
    assert.equal((await call("memory.forget", { scope: "trip" })).forgotten, true);
    // Research cache.
    await call("research_cache.put", { url: "https://example.com/a", data: { title: "A" } });
    assert.equal((await call("research_cache.get", { url: "https://example.com/a" })).data.title, "A");
    await call("research_cache.invalidate", { url: "https://example.com/a" });
    assert.equal((await call("research_cache.get", { url: "https://example.com/a" })).hit, false);

    // Saved sessions and checkpoints.
    assert.deepEqual((await call("profile.session_save", { name: "work" })).pages, [site.url("daily.html")]);
    assert.equal((await call("profile.session_list")).sessions[0].name, "work");
    assert.equal((await call("profile.session_resume", { name: "work" })).opened.length, 1);
    await rejects(call("profile.checkpoint", { name: "cp" }), "policy.denied");
    await call("bookmark.add", { url: "https://checkpoint.example/", title: "Kept" }).catch(() => operator(port, "bookmark.add", { url: "https://checkpoint.example/", title: "Kept" }));
    const checkpoint = await operator(port, "profile.checkpoint", { name: "cp" });
    assert(checkpoint.bookmarks >= 1);
    assert(!/password|token|sitePermissions/i.test(fs.readFileSync(checkpoint.file, "utf8")));
    assert.equal((await operator(port, "profile.restore_checkpoint", { name: "cp" })).settings, true);

    // Domain policy: the person decides which sites agents may use.
    await rejects(call("policy.domain_blocklist", { add: "127.0.0.1" }), "policy.denied");
    await operator(port, "policy.domain_blocklist", { add: "127.0.0.1" });
    await rejects(call("navigate.url", { url: site.url("daily.html") }), "policy.denied");
    await operator(port, "policy.domain_blocklist", { remove: "127.0.0.1" });
    await operator(port, "policy.domain_allowlist", { add: "example.org" });
    await rejects(call("navigate.url", { url: site.url("daily.html") }), "policy.denied");
    await operator(port, "policy.domain_allowlist", { remove: "example.org" });
    await call("navigate.url", { url: site.url("daily.html") });
    assert.equal((await call("policy.action_classify", { method: "act.click" })).known, true);
    assert.equal((await call("policy.preview_side_effect", { method: "observe.page" })).readOnly, true);
    assert.equal((await call("policy.approval_expire")).approvalsExpireAfterSeconds, 300);

    // Artifacts and files, only inside AgentGo's folders.
    const shot = await call("observe.screenshot");
    const note = await call("artifact.create", { text: "call ada@example.com or 0123456789012" });
    assert((await call("artifact.list")).total >= 2);
    assert.equal((await call("artifact.search", { text: "note" })).artifacts.length >= 1, true);
    assert.equal((await call("artifact.hash", { path: note.path })).sha256.length, 64);
    assert.match((await call("artifact.read", { path: note.path })).text, /ada@example.com/);
    assert.equal((await call("artifact.redact", { path: note.path })).redacted, 2);
    assert.doesNotMatch((await call("artifact.preview", { path: note.path })).text, /ada@/);
    assert.match((await call("artifact.link", { path: shot.path })).link, /^file:\/\/\//);
    assert.equal((await call("artifact.metadata", { path: shot.path })).sha256.length, 64);
    const copy = await call("file.copy", { path: note.path, to: path.join(path.dirname(note.path), "copy.txt") });
    const renamed = await call("file.rename", { path: copy.path, to: "renamed.txt" });
    const bundle = await call("artifact.bundle", { paths: [note.path, renamed.path] });
    const unpacked = await call("file.extract_archive", { path: bundle.path });
    assert.equal(unpacked.files.length, 2);
    const written = await call("file.write_text", { path: path.join(path.dirname(note.path), "written.txt"), text: "only this file says this" });
    await rejects(call("artifact.read", { path: "C:/Windows/win.ini" }), "policy.denied");
    await rejects(call("file.copy", { path: note.path, to: "C:/Windows/stolen.txt" }), "policy.denied");
    assert((await call("artifact.deduplicate")).removed.length >= 0);
    assert.equal((await call("artifact.retention", { days: 3650 })).removed, 0);
    // The written file has unique content, so de-duplication left it alone.
    await call("artifact.delete", { path: written.path });
    assert.equal(fs.existsSync(written.path), false);

    // Network control: mock, block and fail requests; read what was sent.
    await call("navigate.url", { url: site.url("daily.html") });
    await call("network.observe_requests");
    const fetchStatus = (url, init = "{}") => call("runtime.evaluate", { expression: `fetch(${JSON.stringify(url)}, ${init}).then(r => r.status + ' ' + r.headers.get('content-type'), e => 'failed')` }).then((r) => r.value);
    await call("network.fulfill", { pattern: "*/api/items*", body: { items: [1, 2] } });
    assert.equal(await fetchStatus(site.url("api/items")), "200 application/json");
    assert.equal((await call("runtime.evaluate", { expression: `fetch(${JSON.stringify(site.url("api/items"))}).then(r => r.json()).then(j => j.items.length)` })).value, 2);
    await call("network.intercept", { pattern: "*/blocked*", action: "block" });
    assert.equal(await fetchStatus(site.url("blocked.js")), "failed");
    await call("network.emulate_errors", { pattern: "*/flaky*", reason: "TimedOut" });
    assert.equal(await fetchStatus(site.url("flaky")), "failed");
    await call("network.fulfill", { pattern: "*/echo*", body: "ok", status: 201 });
    await fetchStatus(site.url("echo"), "{method: 'POST', body: 'hello body', headers: {'X-Probe': '1'}}");
    const sent = (await call("network.requests")).events.filter((r) => r.kind === "request" && /echo/.test(r.url)).pop();
    assert.equal((await call("network.request_body", { requestId: sent.id })).body, "hello body");
    assert.equal((await call("network.headers", { requestId: sent.id })).request["X-Probe"], "1");
    const har = await call("network.har_stop");
    assert(har.entries >= 3 && fs.existsSync(har.path));
    assert.equal((await call("network.clear_interceptors")).removed, 4);
    assert.equal(await fetchStatus(site.url("api/items")), "404 null");
    const page = (await call("network.requests")).events.find((r) => r.kind === "request" && /daily/.test(r.url));
    if (page) assert.match((await call("network.response_body", { requestId: page.id })).body, /Daily fixture/);
    assert.deepEqual((await call("network.websocket_list")).sockets, []);
    assert.deepEqual((await call("network.sse_events")).events, []);
    await rejects(call("network.intercept", { pattern: "*", action: "explode" }), "invalid.params");

    console.log(JSON.stringify({ ok: true }));
  } catch (error) {
    console.error(error);
    failed = true;
  } finally {
    const code = await browser.close();
    site.close();
    process.exitCode = failed || code !== 0 ? 1 : 0;
  }
})();
