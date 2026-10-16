# Handoff from the Sonnet session (2026-10-06)

The owner stopped this session after I caused problems on their real desktop
(section 0). This file says what is done, what is not, what is broken, and
where to start. Read it first, then `docs/HANDOFF.md` (older; its milestone
table is still mostly right) and `docs/issues-log.md`.

Repository: `C:\Users\Admin\Documents\Codex\2026-09-07\yo\work\agentgo`.
**Nothing is committed** (one commit, "Milestone 0"). Commit only if the owner asks.

## 0. Rules (these cost the owner time; do not repeat them)

1. **Never open the browser on the owner's real desktop to test.** I did it
   twice (`target\real-check.ps1`, `target\real-check2.ps1`) to look for the
   taskbar button. The second run showed that AgentGo **takes the foreground**
   from the owner's window, which is exactly what they forbade. Those scripts
   are still in `target\`. Do not run them unless the owner says yes.
   Test through `node scripts\dev\run-integration.js <name>` or
   `target\debug\agentgo-tool.exe quiet-run -- <command>`: both use a private
   desktop, so nothing appears on the real one. `tests/integration/harness.js`
   refuses to start a browser outside quiet-run.
2. No commits. No CAPTCHA solving or bypass (detect it and hand off to the human).
3. Windows PowerShell 5.1: no `&&`; put scripts in files; do not merge native
   stderr with `2>&1` (it prints a NativeCommandError and exit code 1 even when
   the program succeeded; read `$LASTEXITCODE`).
4. The owner's M12 requirement: AgentGo stays **visible and openable** (normal
   window, taskbar button) but **never takes focus, cursor or keyboard**.
   Agents use virtual (DevTools) input only, and the human's own input always wins.
5. Clean-up check at the end of the session: no `agentgo`, `agentgo-tool` or
   `dbg-*` node processes were left running. `%TEMP%\agentgo-realcheck-*`
   folders may remain; they are safe to delete.

## 1. State of the tree

- The last `scripts\build.ps1 -Runtime` succeeded with no warnings
  (`target\debug\agentgo.exe` and `agentgo-tool.exe`). It also regenerates
  `schemas/capability-manifest.json`.
- **Not re-run since the M12.1/M12.2 edits:** `cargo test --workspace --offline`
  (was 122 unit + 17 runtime tests), `cargo clippy ... -D warnings`,
  `cargo test -p agentgo-app --test lockstep` (10 passed before `window.status`
  was registered) and the offline integration suite (8/8 passed with 0 leaks
  *before* these edits). Regressions are possible; see P0-2.

## 2. Finished and verified

**Earlier this session (details in `docs/issues-log.md`):**
- Batch A: Cloudflare-style interstitial detection (detect and hand off only,
  test `site_signals_e2e.js`), loading-state fix, window and exe icon,
  keyboard-shortcut handler (it still has temporary debug logging), the
  `agentgo-tool` binary split, "heading missed on example.com" turned out to be
  invalid (the fixture test proves headings survive).
- Quiet testing (M12 part 0): `agentgo-tool quiet-run` runs a command on a private
  desktop inside a job object, reports any window that leaks to the real desktop
  and exits 97 if one does (`crates/agentgo-app/src/quiet.rs`). The harness has
  `requireQuiet()`, the runner runs every test under quiet-run, tests set
  `AGENTGO_NO_NOTIFY=1`. Live-network tests are marked `// @live` and are skipped
  unless you pass `--live` or set `AGENTGO_LIVE=1`. Default offline suite: 8 tests.

**Verified on the private desktop with the current build:**
- `agentgo-tool probe windows | watch | decoy` work (section 6).
- The agent-launched window stays **below** a full-screen decoy window, is
  visible, not minimized, and has a taskbar-eligible style set.
- Pages stay live behind a covering window: 58 to 61 frames/s and 20 to 21 timer
  ticks/s measured while fully covered. Screenshots of the active tab work
  (about 150 to 190 ms).
- A tab an agent opens in the background loads, and `runtime.evaluate` runs in it.
- `tabs.activate` replies `applied:false` with a reason in `agentTabs=background`
  and `applied:true` in `follow`. An agent calling `settings.set {agentTabs}`
  gets `policy.denied`.
- Window title follows the visible tab: "`<tab title> - AgentGo`".
- On the **real** taskbar (read with UI Automation) a button
  "agentgo - 1 running window" appeared after the `WS_EX_APPWINDOW` fix.

## 3. Coded, but broken or unverified

### P0-1. The agent-launched window still takes the foreground
Real-desktop evidence (`real-check2.ps1`): the owner's window was foreground;
1.4 s after launch the foreground became AgentGo's window; at the end AgentGo was
still foreground **and** at the very bottom of the z-order (position 341 of 564).
That is the worst case: focus stolen and the window invisible.

On the private desktop every variant showed `activeInQueue=true focusInQueue=true`
for the AgentGo window. I first read that as "per-queue state, inconclusive".
**That was wrong:** a window that was never activated has both `false` (the
decoy keeps `true`). So `activeInQueue`/`focusInQueue` of the AgentGo window is
a valid private-desktop proxy for "was activated", and it is currently `true`.

What I tried (code: `show_without_activation` in
`crates/agentgo-cef/src/runtime/app.rs`, called from `on_window_created`):
1. Win32 `ShowWindow(SW_SHOWNOACTIVATE)` + `SetWindowPos(HWND_BOTTOM)` without
   Views `show()`: still activated, and the first `Page.captureScreenshot`
   hung 12 to 20 s because Views never saw a show. Rejected.
2. `WS_EX_NOACTIVATE` set, `window.show()`, repeat `ShowWindow` if still invisible,
   `HWND_BOTTOM`, remove `NOACTIVATE`: screenshots instant, pages live, but the
   shell left the window out of the taskbar and Alt+Tab (it decides at show time
   and never looks again). This is why the owner said "I don't see agentgo
   in my taskbar or anywhere".
3. (current) As 2, with `WS_EX_APPWINDOW` set first and kept: the taskbar button
   exists. **Still activated.**

Because variant 1 never called `Window::show()` and was still activated, the
cause is code that runs after window creation and calls an explicit activation
(Chromium's `HWNDMessageHandler::Activate()` calls `SetForegroundWindow`, which
ignores `WS_EX_NOACTIVATE`). Suspects, none confirmed: initial focus of a
BrowserView or WebContents; `host.set_focus(1)` in `state.rs` (`focus_page`, and
`shell_shortcut` for `focus_address`); `address.focus()` in `ui/chrome.js`;
`autofocus` on `#omniInput` in `ui/index.html`; CEF's own first-focus logic.

How to find it safely (private desktop only): `target\dbg-timeline.js` starts a
decoy, `probe watch`, and a browser, then merges the watch lines with the
browser's `AG-EVENT` log lines by timestamp. My first run printed nothing and
the second was cancelled, so the script itself may be wrong (it waits on the
watcher's `exit` event after attaching late). Fix it or rewrite it, then bisect by
disabling candidate focus calls in background mode. Done when the AgentGo
window shows `activeInQueue=false focusInQueue=false` while the decoy stays
`true`, screenshots still work, and pages stay live.
Fallbacks if no clean cause exists: subclass the HWND and refuse or undo
activation; as a last resort capture the previous foreground window before
showing and call `SetForegroundWindow(previous)` right after (flicker, and keys
typed in between are lost).
The final proof still needs one real-desktop run (foreground timeline plus
taskbar button). Ask the owner first, or let them run it.

### P0-2. Possible click regression: viewport reads 0 x 0
`target\dbg-bg.js` (background presentation, `agentTabs=background`) found
`innerWidth = innerHeight = clientWidth = 0` and `document.elementFromPoint(40,60) === null`
in **both** the active tab and the background tab. `act.click` on the background
tab fails with `target.blocked: ... is covered by nothing at that point`.
I never ran `act.click` on the **active** tab in background presentation after my
edits. If it also fails, every agent click is broken in background mode.
Run `agent_e2e.js` first. Suspects: the new `layout_composition` code in `app.rs`
(hidden tabs get the visible tab's bounds; it did not fix the background tab),
the Views-based inactive show, `Emulation.setFocusEmulationEnabled` in
`cdp.rs::enable`, or the "stay live" switches. If background tabs stay 0 x 0, an
option is `Emulation.setDeviceMetricsOverride` on hidden tabs, cleared on activation.

### P1
- `window.status` is registered in `crates/agentgo-shell-bridge/src/methods.rs`
  (milestone 12, test id `integration.presentation_e2e`) but
  **`tests/integration/presentation_e2e.js` does not exist yet.** The lockstep
  or registry tests probably fail until it does. The milestone bound test is now
  `1..=12`. Fields: `visible, active, foreground, minimized, maximized,
  fullscreen, presentation, humanActive, agentTabs, bounds, activeTabId`.
- Taskbar label is lowercase "agentgo" (taken from the exe). Set ProductName and
  FileDescription to "AgentGo" in `crates/agentgo-app/build.rs`.
- z-order policy is `HWND_BOTTOM`. After P0-1 is fixed, ask the owner: bottom,
  directly behind the foreground window, or normal.
- `browser.shutdown` over stdin did not end the browser within 20 s (plus 10 s
  after closing stdin) on the real-desktop run, so I killed it. The harness waits
  15 s, then kills. Old "shutdown latency" issue; unresolved. Risk: orphaned CEF
  children and locked temp profiles.
- `observe.screenshot` of a non-visible tab returns `target.blocked`
  (`page.rs::screenshot`). Decide if that is acceptable in background mode.

### P2
- The `_byAgent` spoof check was inconclusive (my run was in `follow` mode). Test
  it in `background` mode: `tabs.create {_byAgent:false}` must still open in
  the background. `dispatch.rs::run_core` strips and re-stamps it for
  `tabs.create|tab.create|tabs.activate|tab.activate` only.
- `human_active()` (`state.rs`: a window of ours is foreground **and** the last
  real input was under 4 s ago) is always false on the private desktop, so tests
  see `auto` act like `follow`. Its real-desktop behavior is untested.
- No Settings UI for `agentTabs`, and no operator-only `window.present`.
- Leftover debug output: `AG-DEBUG rawkey` and `AG-RUNTIME shortcut` logging
  in the keyboard handler; `eprintln!` lines behind `AGENTGO_CDP_TRACE` in
  `cdp.rs` and `dispatch.rs`.

## 4. Milestone status

From `docs/HANDOFF.md` section 3, updated. Still-true gaps are summarised.

| Milestone | Status |
|---|---|
| 0 Repo, toolchain, contracts | Done |
| 1 CEF composition | Done for tested paths. Desktop launch by hand, `--smoke` and the sandbox package were not re-tested |
| 2 Product core | Mostly done. Missing: multi-window, session restore, find-in-page, zoom, permissions UI, crash/error pages |
| 3 Protocol and isolation | Gate met (`sessions_e2e`) |
| 4 Human-equivalent control | Core done. Missing: touch/pen, clipboard, drag-and-drop, IME, native file dialogs |
| 5 Observation | Core done. Missing: cross-origin iframes, OCR/canvas, AX-tree view, visual diff, `locate.*`/`ground.*` |
| 6 Workflows | Partly. `workflow.search_extract` is planned: `agent.js` has no `searchResults`. No checkpoints or replay |
| 7 Policy, approvals, handoff | Core done. Missing: DPAPI credential vault, autofill, login/2FA detection, approval-expiry UI. "User input wins" is only started (`human_active`) |
| 8 Artifacts, native, downloads | Small part. Missing: retention, redaction, UIA lane, scan, quarantine |
| 9 Skills, adapters | Partly. CLI and MCP done. `skills/agentgo/SKILL.md` is stale (says Milestones 0 to 2) |
| 10 Daily-browser P1/P2 | Not started |
| 11 Performance, security, release | Not started |
| **12 Quiet, background, accounts** | 12.0 quiet testing: done. 12.1/12.2 background window and independent input: coded, builds, **P0-1 and P0-2 open**. 12.3 Account Center and 12.4 MCP connectors: **not started, no code** |

## 5. M12.3 and M12.4: what the owner asked for (nothing built yet)

Account Center, so agents can use accounts the human links:
- Providers: Google, Yandex, GitHub, LinkedIn (more later).
- Sign-in by OAuth2 + PKCE (S256), `state` and `nonce`, strict redirect-URI
  validation (exact match, loopback on an ephemeral port).
- Tokens go in a DPAPI-encrypted vault and are never shown to pages or agents.
  Sign-in runs in a trusted tab that agents cannot drive.
- A clear consent screen that says the agent gets **100% control** of what that
  account can do. Unlink and revoke must work.
- MCP connectors per account: Google gives Drive, Photos, Gmail; LinkedIn gives
  jobs. Agents call gateway tools and never see a token.
- Tests: a mock OAuth provider plus attack cases (state mismatch, redirect
  tampering, PKCE downgrade, token leakage to agents, replay).

Suggested shape (a proposal, not a decision): new crate `agentgo-accounts`
(provider catalog, PKCE with `sha2` and `getrandom`, redirect validation, vault
trait with a DPAPI implementation, loopback listener), protocol methods registered
in `methods.rs` at milestone 12 (`accounts.*`, `connectors.*`), and a panel in
`ui/tools.*`. Operator-only methods must follow the existing pattern
(`OPERATOR_SETTINGS` and the caller checks in `dispatch.rs`).

## 6. Tools

Build: `powershell -NoProfile -ExecutionPolicy Bypass -File scripts\build.ps1 -Runtime *> target\build.log`
(incremental about 20 to 50 s).
Tests: `cargo test --workspace --offline`; `node scripts\dev\run-integration.js [names] [--live]`.

`agentgo-tool probe ...` (run under quiet-run so it sees only the private desktop):
- `probe windows`: JSON of top-level windows, front to back: `z, title, class,
  pid, visible, minimized, topmost, toolWindow, appWindow, noActivate, owned,
  taskbar, rect, activeInQueue, focusInQueue`.
- `probe decoy [--cover] [--title T]`: a stand-in for the owner's window; prints
  `decoy-ready <hwnd>`.
- `probe watch --seconds N --class C`: prints a line on every change, stamped
  with epoch milliseconds (the same clock as the browser's `AG-EVENT` lines).

Run flags and env: `agentgo run --foreground` (open in front, the old behavior;
agent launches via stdio or `--smoke` are background by default),
`AGENTGO_DATA_ROOT`, `AGENTGO_NO_NOTIFY=1`, `AGENTGO_REMOTE_DEBUGGING_PORT`
(tests use `launch({debugPort})`), `AGENTGO_CDP_TRACE=1`, `AGENTGO_TEST_VISIBLE=1`.
Operator-only settings need the trusted chrome: `operator(port, "settings.set",
{settings:{agentTabs:"background"}})` from the harness. `runtime.evaluate`
needs `--mode full --allow-evaluate`.

Throwaway scripts in `target\` (ignored by git): `dbg-present.js`, `dbg-shot.js`,
`dbg-bg.js`, `dbg-taskbar.js`, `dbg-timeline.js` (all quiet-run only), and
`real-check.ps1`, `real-check2.ps1` (**real desktop, see section 0**). The
second one records the foreground timeline and reads the taskbar through UI
Automation (`Shell_TrayWnd` descendants of type Button).

Where the M12 code lives:
- `crates/agentgo-cef/src/runtime/state.rs`: `RunOptions.foreground` and
  `background()`, `Placement`, `human_active`, `agent_tabs_mode`,
  `agent_tab_placement`, `activate_tab_for_agent`, `window_state`,
  `refresh_window_title`, `create_content_tab_placed`.
- `.../dispatch.rs`: `window.status` is read-only; `agentTabs` is in
  `OPERATOR_SETTINGS`; `_byAgent` stamping in `run_core`.
- `.../app.rs`: `show_without_activation`, the stay-live Chromium switches
  (set through `AGENTGO_PAGES_STAY_LIVE`), `layout_composition`.
- `.../cdp.rs`: focus emulation when running in the background.
- `crates/agentgo-app/src/{quiet,probe}.rs`, `crates/agentgo-cli/src/lib.rs`
  (`quiet-run`, `probe`, `run --foreground`), `tests/fixtures/pages/live.html`
  (frame and timer counters).

## 7. Start here, in order

1. Rebuild. Run `cargo test --workspace --offline`, then
   `node scripts\dev\run-integration.js`. Fix regressions first, starting with
   whether `act.click` works on the active tab (P0-2).
2. Find what activates the window (P0-1) using only the private desktop.
3. Write `tests/integration/presentation_e2e.js`. It should check: a decoy stays
   active and in front; the AgentGo window is visible and below it with
   `activeInQueue=false focusInQueue=false` and `taskbar=true`; `window.status`
   fields; pages live (frames and timers) and a screenshot works while covered;
   `agentTabs=background` makes `tabs.create` leave the active tab alone and
   `tabs.activate` return `applied:false`; `follow` switches; an agent cannot set
   `agentTabs` or spoof `_byAgent`; `--foreground` opens above the decoy.
   Then rerun lockstep and regenerate the manifest.
4. P1 items (taskbar name, z-order decision, shutdown latency). Ask the owner
   about z-order, and before creating any Start-menu or desktop shortcut (none
   exists; the only way to open the app today is `target\debug\agentgo.exe`).
5. M12.3 and M12.4.
6. Trim the debug logging; finish M6 `searchResults`; remaining M4, M5, M7 to M11
   gaps; refresh the stale docs (`milestones.md`, `runtime-status.md`,
   `architecture.md`, `protocol.md`, `skills/agentgo/SKILL.md`, the roadmap's
   "Current position", `docs/HANDOFF.md`, `docs/issues-log.md`).
7. Final validation: `cargo fmt --check`, clippy with `-D warnings`, unit,
   lockstep, and the whole integration suite under quiet-run with 0 leaks.

## 8. Facts learned (so you do not relearn them)

- On the private desktop `GetForegroundWindow()` is not meaningful
  (`window.status.foreground` is always false there). Per-queue activation is.
- Chromium's `Page.captureScreenshot` hangs until the window has been shown
  through Views (`window.show()`); a Win32-only show breaks it.
- The shell decides about the taskbar button and Alt+Tab when a window is shown.
  A `WS_EX_NOACTIVATE` window is left out unless it is also `WS_EX_APPWINDOW`,
  and later style changes are not re-checked.
- A process's first `ShowWindow` call obeys the launcher's STARTUPINFO. The
  harness uses `windowsHide: true`, so the first call is swallowed (hidden); repeat
  the call if `IsWindowVisible` is false.
- CEF's `ShowState` has no "inactive" value and `WindowDelegate` has no
  `can_activate`.
- Hidden tabs take no part in the Views layout, so a tab created in the
  background starts at 0 x 0.
- Without the stay-live switches (`CalculateNativeWinOcclusion` off,
  `disable-backgrounding-occluded-windows`, `disable-renderer-backgrounding`,
  `disable-background-timer-throttling`) a covered window stops painting.
- `GetLastInputInfo` is session-wide, so it cannot decide "the human is using
  this window" alone; `human_active` also requires that a window of ours is foreground.
