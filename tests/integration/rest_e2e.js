// The rest of the register: held file pickers and HTTP sign-in, waits,
// OCR and visual grounding, picture diffs, references and snapshots, spell
// checking, tracing and profiling, DNS and workers, media, page runtime,
// input methods, rich and image clipboard, dropping files, downloads,
// credentials, the browser's own lists, shortcuts, user scripts,
// pagination, source comparison, policy scopes, api.act, BrowserGym,
// fonts and the browser lifecycle (instances, recover, budget, update).
const assert = require("node:assert/strict");
const crypto = require("node:crypto");
const fs = require("node:fs");
const http = require("node:http");
const os = require("node:os");
const path = require("node:path");
const { execFileSync } = require("node:child_process");
const { launch, operator, reservePort, rejects, delay, repo } = require("./harness");

const pages = path.join(repo, "tests", "fixtures", "pages");
const uploadDir = fs.mkdtempSync(path.join(os.tmpdir(), "agentgo-rest-up-"));
const upload = path.join(uploadDir, "report.txt");
fs.writeFileSync(upload, "hello");
const second = path.join(uploadDir, "second.txt");
fs.writeFileSync(second, "two");
const install = fs.mkdtempSync(path.join(os.tmpdir(), "agentgo-install-"));
fs.writeFileSync(path.join(install, "agentgo.exe"), "old version");

// A site with pages, a download, a password-protected page and an update.
const staged = fs.mkdtempSync(path.join(os.tmpdir(), "agentgo-staged-"));
fs.writeFileSync(path.join(staged, "agentgo.exe"), "new version");
fs.writeFileSync(path.join(staged, "README.txt"), "9.9.9");
const zip = path.join(os.tmpdir(), `agentgo-update-${process.pid}.zip`);
execFileSync("powershell", ["-NoProfile", "-Command", `Compress-Archive -Path '${staged}\\*' -DestinationPath '${zip}' -Force`], { windowsHide: true });
const zipBytes = fs.readFileSync(zip);
const sha = crypto.createHash("sha256").update(zipBytes).digest("hex");
let manifest = {};
// Debug builds accept a test key; the real release key never signs test manifests.
const updateKeys = crypto.generateKeyPairSync("ed25519");
process.env.AGENTGO_UPDATE_PUBKEY = updateKeys.publicKey.export({ type: "spki", format: "der" }).subarray(-32).toString("hex");
const signed = (m) => ({ ...m, signature: crypto.sign(null, Buffer.from(`agentgo-update-v1\n${m.version}\n${m.url}\n${String(m.sha256).toLowerCase()}\n`), updateKeys.privateKey).toString("hex") });
const server = http.createServer((request, response) => {
  const url = new URL(request.url, "http://x");
  if (url.pathname === "/secret.html") {
    const auth = request.headers.authorization || "";
    if (auth !== `Basic ${Buffer.from("ada:lovelace").toString("base64")}`) {
      response.writeHead(401, { "WWW-Authenticate": 'Basic realm="Members"' }).end("no");
      return;
    }
    response.writeHead(200, { "Content-Type": "text/html" }).end("<title>Members</title><p>welcome member</p>");
    return;
  }
  if (url.pathname === "/download.txt") { response.writeHead(200, { "Content-Type": "application/octet-stream", "Content-Disposition": "attachment; filename=file.txt" }).end("downloaded content"); return; }
  if (url.pathname === "/manifest.json") { response.writeHead(200, { "Content-Type": "application/json" }).end(JSON.stringify(manifest.unsigned ? manifest : signed(manifest))); return; }
  if (url.pathname === "/update.zip") { response.writeHead(200).end(zipBytes); return; }
  const file = path.join(pages, path.basename(url.pathname) || "rest.html");
  if (!fs.existsSync(file)) { response.writeHead(404).end("missing"); return; }
  response.writeHead(200, { "Content-Type": "text/html; charset=utf-8" }).end(fs.readFileSync(file));
});

(async () => {
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const base = `http://127.0.0.1:${server.address().port}`;
  const port = await reservePort();
  const browser = launch({ args: ["--url", "about:blank", "--upload-dir", uploadDir, "--allow-downloads", "--allow-evaluate"], debugPort: port, env: { AGENTGO_INSTALL_DIR: install } });
  const op = (method, params) => operator(port, method, params);
  const step = (name) => process.stdout.write(`  ${name}\n`);
  try {
    const opened = await browser.call("navigate.url", { url: `${base}/rest.html` });
    const tab = opened.tabId || (await browser.call("tab.status")).tabId;
    const find = (selector) => ({ find: { by: "css", selector } });

    step("file pickers");
    await browser.call("act.click", find("#pick"));
    const picker = await browser.call("wait.dialog", { timeout: 5000 });
    assert.equal(picker.kind, "file");
    assert.equal((await browser.call("dialog.file")).dialog.mode, "open");
    await rejects(browser.call("file.dialog_confirm", { paths: ["C:\\Windows\\win.ini"] }), "policy.denied");
    assert.deepEqual((await browser.call("file.dialog_confirm", { paths: [upload] })).chosen, [upload]);
    await browser.call("wait.text", { text: "picked report.txt", timeout: 3000 });
    const pickRef = (await browser.call("locate.css", { selector: "#pick" })).matches[0].ref;
    await browser.call("act.click", { ref: pickRef });
    await browser.call("wait.dialog", { timeout: 5000 });
    assert.equal((await browser.call("file.dialog_cancel")).cancelled, true);
    await rejects(browser.call("file.choose_directory", { paths: [uploadDir] }), "invalid.params");

    step("drop files, IME, clipboard");
    const dropped = await browser.call("act.drag_file", { ...find("#drop"), paths: [upload, second] });
    assert.equal(dropped.dropped, 2);
    await browser.call("wait.text", { text: "dropped report.txt,second.txt", timeout: 3000 });
    await browser.call("file.drag_to_upload", { ...find("#drop"), paths: [second] });
    await rejects(browser.call("act.drag_file", { ...find("#drop"), paths: ["C:\\Windows\\win.ini"] }), "policy.denied");
    await browser.call("keyboard.composition_start", { ...find("#ime"), text: "に" });
    await browser.call("keyboard.composition_update", { text: "にほ" });
    await browser.call("keyboard.composition_end", { text: "日本" });
    const ime = await browser.call("edit.input_method", { ...find("#ime"), reading: "ご", text: "語" });
    assert.equal(ime.committed, "語");
    assert.match((await browser.call("observe.page")).page, /Composer" value="日本語"/);
    await browser.call("clipboard.write_html", { html: "<b>bold</b> words", paste: true, ...find("#editor") });
    assert.equal((await browser.call("clipboard.read")).text, "bold words");
    const png = (await browser.call("observe.screenshot")).path;
    await browser.call("clipboard.write_image", { path: png, paste: true, ...find("#editor") });
    await browser.call("wait.text", { text: "pasted image image/", timeout: 3000 });
    assert.ok((await browser.call("clipboard.read_image")).bytes > 100);

    step("OCR, visual grounding, diffs");
    const read = await browser.call("observe.ocr");
    assert.match(read.text, /Quarterly/i);
    const spot = await browser.call("ground.visual", { text: "Quarterly Report" });
    assert.ok(spot.x > 0 && spot.y > 0);
    const baseline = await browser.call("observe.visual_diff");
    assert.ok(baseline.baseline);
    await browser.call("act.click", { x: 5, y: 5 });
    const change = await browser.call("observe.visual_diff");
    assert.ok(typeof change.changed === "number");
    assert.ok(Array.isArray((await browser.call("observe.paint_regions", { ms: 200 })).regions));

    step("references and snapshots");
    const ref = (await browser.call("locate.css", { selector: "#next" })).matches[0].ref;
    await browser.call("state.snapshot_store", { name: "before" });
    assert.ok((await browser.call("state.ref_expire")).expired >= 1);
    await rejects(browser.call("act.hover", { ref }), "target.stale");
    const loaded = await browser.call("state.snapshot_load", { name: "before" });
    assert.equal(loaded.sameUrl, true);

    step("spellcheck, tracing, profiling, coverage");
    const spelling = await browser.call("edit.spellcheck", find("#notes"));
    assert.ok(spelling.errors.some((e) => e.word === "sentense" && e.suggestions.includes("sentence")));
    assert.equal((await browser.call("edit.spellcheck", { text: "helo" })).count, 1);
    await browser.call("debug.trace_start");
    await browser.call("act.scroll", { deltaY: 200 });
    const trace = await browser.call("debug.trace_stop");
    assert.ok(trace.events > 0 && fs.existsSync(trace.path));
    assert.ok((await browser.call("debug.trace_export")).top.length > 0);
    assert.ok((await browser.call("debug.profile_cpu", { ms: 300 })).path);
    const coverage = await browser.call("debug.coverage", { ms: 100 });
    assert.ok(Array.isArray(coverage.scripts));

    step("network, workers, screen reader, media");
    const dns = await browser.call("network.dns_status", { host: "localhost" });
    assert.equal(dns.resolved, true);
    assert.ok(Array.isArray((await browser.call("network.service_worker_events")).events));
    assert.ok(Array.isArray((await browser.call("network.background_tasks")).events));
    assert.ok((await browser.call("network.cache_status")).resources >= 1);
    assert.equal((await browser.call("a11y.screen_reader_mode", { on: true })).screenReaderMode, true);
    await browser.call("a11y.screen_reader_mode", { on: false });
    const frame = await browser.call("media.capture_frame", find("#art"));
    assert.equal(frame.width, 120);
    assert.equal((await browser.call("media.fullscreen", find("#art"))).shown, true);
    assert.equal((await browser.call("dialog.fullscreen")).fullscreen, true);
    await browser.call("dialog.fullscreen", { exit: true });
    await rejects(browser.call("media.picture_in_picture", find("#art")), "target.blocked");

    step("page runtime");
    assert.equal((await browser.call("runtime.call_function", { function: "(a, b) => a + b", args: [2, 3] })).value, 5);
    assert.equal((await browser.call("runtime.call_function", { ...find("#big"), function: "(el) => el.textContent" })).value, "Quarterly Report");
    const init = await browser.call("runtime.add_init_script", { source: "window.__early = 42" });
    await browser.call("runtime.add_binding", { name: "toAgent" });
    await browser.call("navigate.url", { url: `${base}/rest.html` });
    assert.equal((await browser.call("runtime.call_function", { function: "() => window.__early" })).value, 42);
    await browser.call("runtime.call_function", { function: "() => window.toAgent('hi agent')" });
    await delay(200);
    const messages = await browser.call("runtime.message", { data: { hello: 1 } });
    assert.ok(messages.received.some((m) => m.payload === "hi agent"));
    await browser.call("wait.text", { text: 'message {"hello":1}', timeout: 2000 });
    await browser.call("runtime.remove_init_script", { identifier: init.identifier });
    await browser.call("runtime.remove_binding", { name: "toAgent" });
    await rejects(browser.call("runtime.add_binding", { name: "__agReport" }), "invalid.params");

    step("popups and navigation");
    const popupWait = browser.call("wait.popup", { timeout: 5000 });
    await browser.call("act.click", find("#popup"));
    const popup = await popupWait;
    assert.ok(popup.tabId);
    await browser.call("tabs.close", { tabId: popup.tabId });
    assert.equal(typeof (await browser.call("navigate.prerender_status")).prerendered, "boolean");
    assert.equal((await browser.call("navigate.resume_after_interruption")).resumed, false);

    step("HTTP sign-in");
    await browser.call("tabs.navigate", { tabId: tab, url: `${base}/secret.html` });
    const auth = await browser.call("wait.dialog", { tabId: tab, timeout: 6000 });
    assert.equal(auth.kind, "auth");
    assert.equal((await browser.call("dialog.auth", { tabId: tab })).prompt.realm, "Members");
    await rejects(browser.call("dialog.auth", { tabId: tab, username: "ada", password: "x" }), "policy.denied");
    await op("dialog.auth", { tabId: tab, username: "ada", password: "lovelace" });
    await browser.call("wait.text", { tabId: tab, text: "welcome member", timeout: 5000 });

    step("downloads");
    await browser.call("navigate.url", { url: `${base}/rest.html` });
    const got = await browser.call("act.download", { ...find("#dl"), timeout: 15000 });
    assert.equal(got.complete, true, JSON.stringify(got));
    const file = got.download.path;
    assert.equal((await browser.call("download.quarantine", { path: file })).marked, true);
    await browser.call("act.download", { url: `${base}/download.txt` });
    const dupes = await browser.call("download.deduplicate");
    assert.ok(dupes.duplicates.length >= 1);
    await rejects(browser.call("download.deduplicate", { remove: true }), "policy.denied");
    assert.ok((await browser.call("download_manager.search", { text: "file" })).items.length >= 1);
    assert.equal((await browser.call("download_manager.remove", { id: got.download.id })).removed, true);
    assert.ok(Array.isArray((await browser.call("download.resume_after_restart")).interrupted));

    step("credentials");
    const redacted = await browser.call("credential.redact", { text: "password=hunter2 mail ada@example.com" });
    assert.ok(!redacted.text.includes("hunter2") && !redacted.text.includes("ada@example.com"));
    await rejects(browser.call("credential.revoke_session"), "policy.denied");
    const side = await browser.call("profile.open", { profile: "rest", url: `${base}/rest.html` });
    await browser.call("wait.load", { tabId: side.tabId });
    await browser.call("storage.cookie_set", { tabId: side.tabId, name: "sid", value: "1" });
    assert.ok((await browser.call("credential.revoke_session", { tabId: side.tabId })).cookiesRemoved >= 1);

    step("bookmarks, history, shortcuts, fonts");
    await browser.call("bookmark.add", { url: `${base}/a`, title: "A" });
    await browser.call("bookmark.add", { url: `${base}/b`, title: "B" });
    assert.equal((await browser.call("bookmark.edit", { url: `${base}/a`, title: "A edited" })).edited, true);
    assert.equal((await browser.call("bookmark.move", { url: `${base}/a`, index: 0 })).order[0], `${base}/a`);
    assert.equal((await browser.call("bookmark.list")).items[0].title, "A edited");
    await rejects(browser.call("history.delete", { contains: "rest" }), "policy.denied");
    assert.ok((await op("history.delete", { contains: "secret" })).deleted >= 0);
    await rejects(browser.call("browser_shortcut.configure", { action: "history", keys: "Ctrl+K" }), "policy.denied");
    assert.equal((await op("browser_shortcut.configure", { action: "history", keys: "ctrl+k" })).shortcuts["Ctrl+K"], "history");
    await rejects(op("browser_shortcut.configure", { action: "history", keys: "k" }), "invalid.params");
    await rejects(browser.call("appearance.fonts", { size: 18 }), "policy.denied");
    assert.equal((await browser.call("appearance.fonts", { profile: "rest", serif: "Georgia", size: 18 })).set.size, 18);
    assert.ok((await browser.call("browser_ui.translate", { to: "de" }).catch((e) => ({ error: e.code }))) !== undefined);

    step("user scripts");
    await rejects(browser.call("script.user_install", { name: "mark", matches: [`${base}/*`], code: "document.body.dataset.marked = 'yes'" }), "policy.denied");
    await op("script.user_install", { name: "mark", matches: ["http://127.0.0.1/*"], code: "document.title = 'marked by user script'" });
    assert.ok((await browser.call("script.user_list")).scripts.some((s) => s.name === "mark"));
    await browser.call("navigate.url", { url: `${base}/rest.html` });
    await delay(500);
    assert.equal((await browser.call("tab.status")).title, "marked by user script");
    await browser.call("script.user_disable", { name: "mark" });
    await rejects(browser.call("script.user_enable", { name: "mark" }), "policy.denied");
    await op("script.user_remove", { name: "mark" });

    step("pagination, sources, policy, api, BrowserGym");
    await browser.call("navigate.url", { url: `${base}/rest.html` });
    const pagesRead = await browser.call("collect.paginate", { name: "items", items: { by: "css", selector: "#items a" }, next: { by: "css", selector: "#next" }, pages: 2 });
    assert.equal(pagesRead.results.length, 2, JSON.stringify(pagesRead));
    assert.match(JSON.stringify(pagesRead.results[1]), /Item C2/);
    assert.equal((await browser.call("collect.resume", { name: "items" })).done, 2);
    const compared = await browser.call("research.compare_sources", { urls: [`${base}/rest.html`, `${base}/rest.html?page=3`], terms: ["item"] });
    assert.equal(compared.sources.length, 2);
    for (const scope of ["policy.profile_scope", "policy.file_scope", "policy.network_scope", "policy.permission_scope"]) {
      assert.equal(typeof (await browser.call(scope)), "object");
    }
    assert.ok((await browser.call("policy.file_scope")).uploadLaunchFolders.length === 1);
    await rejects(browser.call("handoff.cancel"), "invalid.params");
    const acted = await browser.call("api.act", { method: "act.click", params: find("#next") });
    assert.equal(acted.ok, true);
    assert.equal((await browser.call("api.act", { method: "wait.text", params: { text: "never", timeout: 100 } })).recovery.class, "timeout");
    const gym = await browser.call("benchmark.browsergym", { action: "goto('" + base + "/rest.html')" });
    assert.match(gym.url, /rest\.html/);
    const bid = gym.axtree_txt.split("\n").find((l) => l.includes("Next")).match(/\[(\d+)\]/)[1];
    const after = await browser.call("benchmark.browsergym", { action: `click('${bid}')` });
    assert.equal(after.last_action_error, null);
    assert.match((await browser.call("benchmark.browsergym", { action: "click('99999')" })).last_action_error, /stale/);

    step("browser lifecycle");
    assert.ok((await browser.call("browser.command_line")).switches.length > 0);
    assert.ok(Array.isArray((await browser.call("browser.feature_flags")).now.enable));
    await rejects(browser.call("browser.feature_flags", { enable: ["X"] }), "policy.denied");
    assert.ok((await browser.call("browser.message_loop_status")).beats > 0);
    assert.ok(Array.isArray((await browser.call("browser.recover")).reloaded));
    assert.ok((await browser.call("browser.resource_budget")).usage.processes >= 1);
    await op("browser.resource_budget", { maxTabs: 50 });
    await rejects(browser.call("browser.suspend"), "policy.denied");
    assert.equal((await op("browser.suspend")).suspended, true);
    assert.equal((await op("browser.resume")).suspended, false);
    assert.equal((await browser.call("browser.low_power_mode", { on: true })).lowPower, true);
    await browser.call("browser.low_power_mode", { on: false });
    assert.ok((await browser.call("browser.remote_instance")).instances.some((i) => i.self));
    assert.ok((await browser.call("browser.attach")).pipe);
    assert.ok(Array.isArray((await browser.call("browser.detach")).released));
    await rejects(op("browser.restart"), "invalid.params");

    step("auto-update");
    manifest = { version: "9.9.9", url: `${base}/update.zip`, sha256: sha, notes: "test" };
    manifest.unsigned = true;
    await rejects(browser.call("browser.update_check", { manifest: `${base}/manifest.json` }), "policy.denied");
    delete manifest.unsigned;
    // Release builds only trust the release key, so the test-key flow runs on debug
    // builds; update_release_e2e covers the release key against the real package.
    if (/release|dist/i.test(process.env.AGENTGO_EXE || "")) {
      await rejects(browser.call("browser.update_check", { manifest: `${base}/manifest.json` }), "policy.denied");
    } else {
    const check = await browser.call("browser.update_check", { manifest: `${base}/manifest.json` });
    assert.equal(check.available, true);
    await rejects(browser.call("browser.update_stage", { manifest: `${base}/manifest.json` }), "policy.denied");
    manifest.sha256 = "0".repeat(64);
    await rejects(op("browser.update_stage", { manifest: `${base}/manifest.json` }), "policy.denied");
    manifest.sha256 = sha;
    const stagedNow = await op("browser.update_stage", { manifest: `${base}/manifest.json` });
    assert.equal(stagedNow.staged, true);
    assert.equal(fs.readFileSync(path.join(stagedNow.path, "agentgo.exe"), "utf8"), "new version");
    // The swap script itself, on the test install (no process to wait for).
    const swap = path.join(repo, "crates", "agentgo-cef", "src", "runtime", "update.ps1");
    execFileSync("powershell", ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", swap, "-Install", install, "-Staged", stagedNow.path, "-Backup", path.join(install, "updates", "previous")], { windowsHide: true });
    assert.equal(fs.readFileSync(path.join(install, "agentgo.exe"), "utf8"), "new version");
    assert.equal(fs.readFileSync(path.join(install, "updates", "previous", "agentgo.exe"), "utf8"), "old version");
    execFileSync("powershell", ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", swap, "-Install", install, "-Staged", path.join(install, "updates", "previous"), "-Backup", path.join(install, "updates", "rolled")], { windowsHide: true });
    assert.equal(fs.readFileSync(path.join(install, "agentgo.exe"), "utf8"), "old version", "rolled back");
    }

    step("another instance");
    const other = await browser.call("browser.launch", { url: "about:blank", ephemeral: true });
    assert.ok(other.instanceId && other.pid);
    // End the whole process tree: Chromium's helper processes outlive a
    // plain kill of the main process and keep the test's job alive.
    try { require("node:child_process").execFileSync("taskkill", ["/PID", String(other.pid), "/T", "/F"], { stdio: "ignore" }); } catch (_) {}
    console.log("rest_e2e: PASS");
  } finally {
    await browser.close();
    server.close();
  }
})().catch((error) => {
  console.error(error);
  process.exit(1);
});
