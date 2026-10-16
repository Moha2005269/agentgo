// End-to-end proof that observation and actions work against a real page:
// compact snapshots with references, trusted input, deltas, shadow DOM,
// frames, stale/ambiguous/covered targets, dialogs, uploads, policy gates.
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { launch, fixture, rejects, refOf } = require("./harness");

const uploadDir = fs.mkdtempSync(path.join(os.tmpdir(), "agentgo-upload-"));
const uploadFile = path.join(uploadDir, "note.txt");
fs.writeFileSync(uploadFile, "hello");
const outsideFile = path.join(os.tmpdir(), `agentgo-outside-${process.pid}.txt`);
fs.writeFileSync(outsideFile, "nope");

(async () => {
  const browser = launch({ args: ["--url", "about:blank", "--upload-dir", uploadDir] });
  try {
    // One call: navigate, wait for load, return the compact page view.
    const opened = await browser.call("navigate.url", { url: fixture("agent.html") });
    assert.equal(opened.loaded, true);
    assert.equal(opened.title, "AgentGo action fixture");
    const page = opened.page;
    assert.match(page, /^# Checkout practice/m);
    assert.match(page, /\[\d+\] textbox "Full name" required/);
    assert.match(page, /\[\d+\] textbox\(email\) "Email"/);
    assert.match(page, /\[\d+\] combobox "Plan" value="Free"/);
    assert.match(page, /\[\d+\] button "Shadow button"/, "open shadow DOM is observed");
    assert.match(page, /\[\d+\] button "Inner frame button"/, "same-origin frames are observed");
    assert(page.length < 1500, `snapshot should be compact, got ${page.length} chars`);

    // Trusted click + delta instead of a full re-observation.
    const counter = refOf(page, "Count clicks");
    const clicked = await browser.call("act.click", { ref: counter });
    assert.match(clicked.target, /button "Count clicks"/);
    assert.equal(clicked.navigated, undefined);
    let text = (await browser.call("observe.text")).text;
    assert.match(text, /counter 1 trusted=true/, "page received a trusted click");

    // Batch: fill a whole form and save in one request.
    const batch = await browser.call("workflow.batch", {
      steps: [
        { method: "act.fill", params: { ref: refOf(page, "Full name"), text: "Ada Lovelace" } },
        { method: "act.fill", params: { ref: refOf(page, '"Email"'), text: "ada@example.com" } },
        { method: "act.select_option", params: { ref: refOf(page, '"Plan"'), value: "Pro tier" } },
        { method: "act.check", params: { ref: refOf(page, "Send me news") } },
        { method: "act.fill", params: { ref: refOf(page, '"Notes"'), text: "typed in a rich editor" } },
        { method: "act.upload", params: { ref: refOf(page, "Attachment"), paths: [uploadFile] } },
        { method: "act.click", params: { ref: refOf(page, "Save draft") } },
      ],
    });
    assert.equal(batch.completed, 7, JSON.stringify(batch));
    assert.equal(batch.failedStep, undefined);
    text = (await browser.call("observe.text")).text;
    assert.match(text, /saved name=Ada Lovelace email=ada@example\.com plan=pro news=true notes=typed in a rich editor file=note\.txt trusted=true/);

    // Nothing changed since the last action: the delta is empty.
    const diff = await browser.call("state.diff");
    assert.equal(diff.changes, "", "nothing changed since the last action");

    // form.fill by label; Enter is a real key event.
    await browser.call("form.fill", { fields: { "Full name": "Grace Hopper" } });
    // Enter in a search box is a real key event and needs no approval...
    await browser.call("act.fill", { ref: refOf(page, "Search the catalog"), text: "blue widgets", submit: true });
    text = (await browser.call("observe.text")).text;
    assert.match(text, /search-enter trusted=true/);
    assert.match(text, /searched blue widgets/);
    // ...but Enter that would submit a consequential form is gated like its
    // submit button.
    await rejects(browser.call("act.press", { key: "Enter", ref: refOf(page, "Full name") }), "approval.required");

    // Upload outside the granted directory is refused.
    await rejects(browser.call("act.upload", { ref: refOf(page, "Attachment"), paths: [outsideFile] }), "policy.denied");

    // Shadow DOM and frame targets receive real clicks.
    await browser.call("act.click", { ref: refOf(page, "Shadow button") });
    await browser.call("act.click", { ref: refOf(page, "Inner frame button") });
    text = (await browser.call("observe.text")).text;
    assert.match(text, /shadow clicked trusted=true/);
    assert.match(text, /frame clicked inner trusted=true/);

    // Dynamic content: the delta reports the new element with a new reference.
    await browser.call("act.click", { ref: refOf(page, "Reveal later") });
    await browser.call("wait.text", { text: "Late arrival", timeout: 5000 });
    const late = await browser.call("state.diff");
    assert.match(late.changes, /^\+ \[\d+\] button "Late arrival"$/m);
    await browser.call("act.click", { ref: Number(late.changes.match(/\[(\d+)\] button "Late arrival"/)[1]) });

    // A covered target is reported, not clicked blindly.
    await browser.call("act.click", { ref: refOf(page, "Cover page") });
    const covered = await rejects(browser.call("act.click", { ref: counter }), "target.blocked");
    assert.match(covered.message, /covered/);
    await browser.call("act.click", { x: 5, y: 5 }); // dismiss the overlay by coordinate
    await browser.call("act.click", { ref: counter });

    // Stale and ambiguous references stop instead of guessing.
    const twin = refOf(page, '"Twin"');
    await browser.call("act.click", { ref: twin });
    await rejects(browser.call("act.click", { ref: twin }), "target.ambiguous");
    await rejects(browser.call("act.click", { ref: 99999 }), "target.stale");

    // Money actions always need the operator, in every mode.
    const money = await rejects(browser.call("act.click", { ref: refOf(page, "Buy now") }), "approval.required");
    assert(money.data.approvalId);
    const pending = await browser.call("approval.wait", { approvalId: money.data.approvalId, timeout: 300 });
    assert.equal(pending.status, "pending");
    // Final form submission needs approval in approval mode.
    await rejects(browser.call("act.click", { ref: refOf(page, "Send profile") }), "approval.required");
    // An agent cannot approve its own request or change operator settings.
    await rejects(browser.call("agent.approval.respond", { approvalId: money.data.approvalId, approve: true }), "policy.denied");
    await rejects(browser.call("settings.set", { settings: { controlMode: "full" } }), "policy.denied");
    await rejects(browser.call("settings.set", { settings: { downloadsEnabled: true } }), "policy.denied");
    await rejects(browser.call("agent.resume"), "policy.denied");
    text = (await browser.call("observe.text")).text;
    assert.doesNotMatch(text, /PURCHASED|submitted/, "gated actions were not performed");

    // JavaScript dialog: reported, blocks actions, answered explicitly.
    const alertClick = await browser.call("act.click", { ref: refOf(page, "Show alert") });
    assert.equal(alertClick.dialog?.message, "Fixture alert");
    await rejects(browser.call("act.click", { ref: counter }), "target.blocked");
    const handled = await browser.call("dialog.accept");
    assert.equal(handled.accepted, true);

    // Scrolling and off-screen targets.
    const scrolled = await browser.call("act.scroll", { to: "bottom" });
    assert(scrolled.scroll.y > 0);
    await browser.call("act.click", { ref: refOf(page, "Bottom button") });
    text = (await browser.call("observe.text")).text;
    assert.match(text, /bottom clicked/);

    // Structured views.
    const forms = await browser.call("observe.forms");
    assert.equal(forms.forms[0].name, "Profile");
    assert(forms.forms[0].fields.some((f) => f.label === "Plan" && f.options.includes("Team")));
    const links = await browser.call("observe.links");
    assert(links.links.some((l) => l.text === "Open the next page"));
    const filtered = await browser.call("observe.page", { filter: "frame" });
    assert.match(filtered.page, /Inner frame button/);
    assert(filtered.page.split("\n").length <= 2);

    // Page script evaluation is refused in approval mode.
    await rejects(browser.call("runtime.evaluate", { expression: "1+1" }), "policy.denied");

    // A screenshot is an artifact handle, not inline bytes.
    const shot = await browser.call("observe.screenshot");
    assert(fs.statSync(shot.path).size === shot.bytes && shot.bytes > 1000);
    assert.equal(shot.base64, undefined);

    // Clicking a link navigates and the reply carries the new page.
    await browser.call("act.scroll", { to: "top" });
    const nav = await browser.call("act.click", { ref: refOf(page, "Open the next page") });
    assert.equal(nav.navigated, true);
    assert.match(nav.url, /fixture\.html$/);
    // References from the previous document are stale, never reused.
    await rejects(browser.call("act.click", { ref: counter }), "target.stale");

    // The audit trail recorded the agent's actions without typed text.
    const status = await browser.call("agent.status");
    assert(status.activity.some((a) => a.method === "act.fill" && a.summary.startsWith("ref=")));
    assert(!JSON.stringify(status.activity).includes("Ada Lovelace"));

    console.log(JSON.stringify({ ok: true, snapshotChars: page.length, snapshotLines: page.split("\n").length }));
  } catch (error) {
    console.error(error); // report the real failure before cleanup
    process.exitCode = 1;
  } finally {
    const code = await browser.close();
    fs.rmSync(uploadDir, { recursive: true, force: true });
    fs.rmSync(outsideFile, { force: true });
    assert.equal(code, 0, `browser exit code ${code}\n${browser.stderr().slice(-1500)}`);
  }
})().catch((error) => { console.error(error); process.exit(1); });
