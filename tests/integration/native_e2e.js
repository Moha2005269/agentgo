// The native Windows lane: AgentGo drives another program's window through
// UI Automation (a WPF window with full patterns, and a classic WinForms
// window that takes window messages). Agents need the person's nativeControl
// setting, and each action needs the person's approval of that action.
const assert = require("node:assert/strict");
const path = require("node:path");
const { spawn } = require("node:child_process");
const { launch, operator, reservePort, rejects, delay, repo } = require("./harness");

(async () => {
  const title = `AgentGo Native ${process.pid}`;
  const fixture = spawn("powershell", ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", path.join(repo, "tests", "fixtures", "native-form.ps1"), "-Title", title], { stdio: "ignore", windowsHide: true });
  const port = await reservePort();
  const browser = launch({ args: ["--url", "about:blank", "--mode", "full"], debugPort: port });
  const op = (method, params = {}) => operator(port, method, { window: title, ...params });
  try {
    await browser.call("browser.health");
    // Settings about the person and AgentGo itself need no permission.
    assert.equal(typeof (await browser.call("native.accessibility")).screenReader, "boolean");
    assert.ok((await browser.call("native.multi_monitor")).monitors.length >= 1);
    assert.ok((await browser.call("window.native_handle")).hwnd > 0);
    assert.equal((await browser.call("native.taskbar", { text: "test" })).flashed, true);
    assert.ok(Array.isArray((await browser.call("native.global_hotkey")).hotkeys));
    await rejects(browser.call("native.global_hotkey", { keys: "Ctrl+Alt+F9", command: "agent.pause" }), "policy.denied");
    const hot = await operator(port, "native.global_hotkey", { keys: "Ctrl+Alt+F9", command: "agent.pause" });
    assert.equal(hot.hotkeys.length, 1);
    assert.equal((await operator(port, "native.global_hotkey", { keys: "Ctrl+Alt+F9", remove: true })).hotkeys.length, 0);

    // Other programs are off limits until the person allows them.
    await rejects(browser.call("native.window_list"), "policy.denied");
    await rejects(browser.call("settings.set", { settings: { nativeControl: true } }), "policy.denied");
    await operator(port, "settings.set", { settings: { nativeControl: true } });

    let windows = [];
    for (let i = 0; i < 40 && !windows.some((w) => w.title === title); i += 1) {
      await delay(500);
      windows = (await browser.call("native.window_list")).windows;
    }
    const target = windows.find((w) => w.title === title);
    assert.ok(target, "the fixture window is listed");
    assert.ok(windows.every((w) => w.agentgo !== true || w.process.startsWith("agentgo")));
    const tree = await browser.call("native.ui_tree", { window: title, depth: 4 });
    assert.ok(tree.elements.some((e) => e.type === "Button" && e.name === "Press"));
    const found = await browser.call("native.find", { window: title, name: "Press", type: "Button" });
    assert.equal(found.matches.length, 1);
    assert.equal((await browser.call("native.read_value", { window: title, name: "Agree" })).value, "Off");

    // Actions need approval of exactly that action.
    const gate = await rejects(browser.call("native.click", { window: title, path: found.matches[0].path }), "approval.required");
    await operator(port, "agent.approval.respond", { approvalId: gate.data.approvalId, approve: true });
    const clicked = await browser.call("native.click", { window: title, path: found.matches[0].path, approvalId: gate.data.approvalId });
    assert.equal(clicked.how, "invoke");
    assert.equal((await browser.call("native.read_value", { window: title, automationId: "Status" })).name, "pressed");

    // The operator's own calls need no approval: every action once.
    assert.equal((await op("native.type", { name: "Entry", text: "hello" })).value, "hello");
    assert.equal((await op("native.type", { name: "Entry", text: " world", append: true })).value, "hello world");
    assert.equal((await op("native.click", { name: "Agree" })).how, "toggle");
    assert.equal((await op("native.read_value", { name: "Agree" })).value, "On");
    assert.equal((await op("native.select", { name: "Color", option: "Green" })).value, "Green");
    assert.equal((await op("native.expand", { name: "Root" })).state, "Expanded");
    assert.equal((await op("native.collapse", { name: "Root" })).state, "Collapsed");
    assert.ok((await op("native.scroll", { name: "Items", direction: "down" })).verticalPercent > 0);
    const moved = await op("native.window_bounds", { x: 60, y: 60, width: 480 });
    assert.equal(moved.bounds.x, 60);
    assert.equal((await op("native.window_snap", { side: "minimize" })).minimized, true);
    assert.equal((await op("native.window_snap", { side: "restore" })).minimized, false);

    // The classic window: window messages, no activation.
    const legacy = `${title} Legacy`;
    assert.equal((await op("native.double_click", { window: legacy, name: "double me" })).how, "messages");
    await delay(300);
    assert.equal((await op("native.find", { window: legacy, name: "doubled" })).matches.length, 1);
    await op("native.key", { window: legacy, class: "EDIT", key: "Enter" });
    await delay(300);
    assert.equal((await op("native.find", { window: legacy, name: "key Return" })).matches.length, 1);
    assert.equal((await op("native.type", { window: legacy, class: "EDIT", text: "abc" })).how, "messages");
    assert.equal((await op("native.read_value", { window: legacy, class: "EDIT" })).name, "abc");

    // Dialogs, tray and clipboard answer (listing needs no approval).
    assert.ok(Array.isArray((await browser.call("native.file_dialog")).dialogs));
    await rejects(browser.call("native.save_dialog", { button: "Save" }), "approval.required");
    await rejects(browser.call("native.clipboard"), "approval.required");
    await rejects(browser.call("native.window_focus", { window: title }), "approval.required");
    await rejects(browser.call("native.hotkey", { window: title, keys: ["ctrl", "a"] }), "approval.required");
    await rejects(browser.call("native.drag_drop", { window: title, name: "Entry", toName: "Items" }), "approval.required");
    await rejects(browser.call("native.system_tray", { name: "Start" }), "approval.required");
    await rejects(browser.call("native.print_dialog", { button: "Print" }), "approval.required");
    await rejects(browser.call("native.permission_dialog", { button: "Yes" }), "approval.required");
    await rejects(browser.call("native.double_click", { window: title, name: "Twice", real: true }), "approval.required");
    assert.ok(Array.isArray((await browser.call("native.print_dialog")).dialogs));
    assert.ok(Array.isArray((await browser.call("native.permission_dialog")).dialogs));
    assert.ok(Array.isArray((await browser.call("native.save_dialog")).dialogs));
    // The system clipboard is the person's: put back what was there.
    const saved = (await op("native.clipboard")).text;
    const clip = await op("native.clipboard", { text: "agentgo clipboard" });
    assert.equal(clip.written, 17);
    assert.equal((await op("native.clipboard")).text, "agentgo clipboard");
    if (saved) await op("native.clipboard", { text: saved });
    assert.equal((await browser.call("native.toast", { title: "AgentGo", text: "test" })).shown, true);
    assert.equal((await browser.call("native.notification", { text: "test" })).notified, true);
    await rejects(browser.call("native.click", { window: title, path: "0.99.99" }), "approval.required");
    await rejects(op("native.click", { path: "0.99.99" }), "target.stale");
    await rejects(op("native.click", { window: "no window has this title" }), "target.stale");
    console.log("native_e2e: ok");
  } finally {
    await browser.close();
    fixture.kill();
    spawn("powershell", ["-NoProfile", "-Command", `Get-Process powershell -ErrorAction SilentlyContinue | Where-Object MainWindowTitle -like '${title}*' | Stop-Process -Force`], { stdio: "ignore", windowsHide: true });
  }
})().catch((error) => { console.error(error); process.exit(1); });
