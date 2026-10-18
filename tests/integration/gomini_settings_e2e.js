// @live GoMini from Settings, the way a person gets it: the card offers the
// download, the button becomes a progress bar, the real files come from
// Hugging Face and GitHub (about 390 MB, checked by SHA-256), the switch
// appears and is on, and a delegated subgoal runs on the new install.
// Screenshots of each state go to target/integration-logs/gomini-*.png.
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { launch, serve, reservePort, operator, cdpEval, cdpTarget, delay, repo } = require("./harness");

async function shot(port, name) {
  const target = await cdpTarget(port, "tools.html");
  const socket = new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => { socket.addEventListener("open", resolve, { once: true }); socket.addEventListener("error", reject, { once: true }); });
  const data = await new Promise((resolve) => {
    socket.addEventListener("message", (event) => { const m = JSON.parse(event.data); if (m.id === 1) resolve(m.result.data); });
    socket.send(JSON.stringify({ id: 1, method: "Page.captureScreenshot", params: { format: "png" } }));
  });
  socket.close();
  const file = path.join(repo, "target", "integration-logs", `gomini-${name}.png`);
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, Buffer.from(data, "base64"));
}

const card = (port) => cdpEval(port, "tools.html", `(() => { const c = document.querySelector('[data-gomini-card]'); c?.scrollIntoView({ block: 'center' }); return c ? c.innerText : null; })()`);

(async () => {
  const site = await serve();
  const port = await reservePort();
  const root = process.env.AGENTGO_GOMINI_FRESH_ROOT || fs.mkdtempSync(path.join(os.tmpdir(), "agentgo-gomini-fresh-"));
  const browser = launch({ args: ["--url", site.url("gomini.html"), "--mode", "full"], debugPort: port, env: { AGENTGO_GOMINI_ROOT: root } });
  let failed = false;
  try {
    await browser.call("tabs.list");
    await operator(port, "ui.open_tool", { tool: "settings" });
    let text = null;
    for (let i = 0; i < 50 && !text; i += 1) { text = await card(port).catch(() => null); if (!text) await delay(100); }
    assert.match(text, /Download GoMini/);
    assert.match(text, /3\d\d MB/);
    await delay(600);
    await shot(port, "1-offer");

    // The person presses Download.
    await cdpEval(port, "tools.html", `document.querySelector('[data-gomini="download"]').click()`);
    let saw = "";
    for (let i = 0; i < 40; i += 1) {
      saw = await card(port);
      if (/% *$|·\s*\d+%/m.test(saw)) break;
      await delay(250);
    }
    assert.match(saw, /\d+%/, `no progress shown: ${saw}`);
    await delay(1500);
    await shot(port, "2-downloading");

    // The real download finishes and checks itself.
    const started = Date.now();
    let status;
    for (;;) {
      status = await browser.call("gomini.status");
      if (!status.download.running) break;
      if (Date.now() - started > 25 * 60_000) throw new Error("download took over 25 minutes");
      await delay(2000);
    }
    assert.equal(status.download.error, null, `download failed: ${status.download.error}`);
    assert.equal(status.installed, true);
    assert.equal(status.enabled, true, "downloading switches GoMini on");
    console.log("gomini_settings_e2e install:", JSON.stringify({ seconds: Math.round((Date.now() - started) / 1000), installation: status.installation }));
    await delay(1500);
    text = await card(port);
    assert.match(text, /Activate GoMini/);
    assert.match(text, /ms per step/);
    await shot(port, "3-active");

    // The new install takes a subgoal.
    for (let i = 0; i < 100 && !(await browser.call("gomini.status")).running; i += 1) await delay(200);
    const result = await browser.call("gomini.delegate", {
      subgoal: "Fill this travel form using Primary City=city; Postal code=postal. Save draft when all required fields match. Never publish.",
      values: { city: "Maple-3321", postal: "11511" },
      completion: "Draft saved successfully with every requested field value",
      constraints: ["Never publish. Never invent a missing value."],
    }, 120_000);
    console.log("gomini_settings_e2e delegate:", JSON.stringify({ status: result.status, steps: result.steps.map((s) => s.choice), modelMs: result.modelMs, ms: result.ms }));
    assert.equal(result.status, "done", JSON.stringify(result));
    const after = await browser.call("decision.record", { subgoal: "check" });
    assert.deepEqual(after.state.status, ["Draft saved successfully."]);
    console.log("gomini_settings_e2e: ok");
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
