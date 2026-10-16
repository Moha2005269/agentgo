# Handoff from the Opus session (2026-10-06)

Read this first. Older files (`docs/sonnethandoff.md`, `docs/HANDOFF.md`,
`docs/STATUS.md`) are superseded where they disagree. Problems and how they
were solved are in `docs/issues-log.md` (section D); add to it as you go, the
owner needs it for the final report.

Repository: `C:\Users\Admin\Documents\Codex\2026-09-07\yo\work\agentgo`.
**Nothing is committed** (one commit, "Milestone 0"). Commit only if asked.

## 0. Rules

1. **Run tests on the owner's real desktop** (owner's instruction,
   2026-10-06: the window stays in the background and they want to watch):
   `AGENTGO_TEST_VISIBLE=1 node scripts\dev\run-integration.js <name>`.
   The private desktop (`agentgo-tool.exe quiet-run`) is no longer the default.
2. No CAPTCHA solving or bypass. Detect and hand off.
3. Shell heredocs break on quotes and backticks. Write edit scripts with the
   file tool (`target\tmp\editN.js`), then `node` them.
4. A test that "times out" may be a crashed browser. The harness prints the
   exit code; Chromium CHECK failures exit `0x80000003` with no text.
5. The owner interrupts long commands. Run tests one or two at a time.

## 1. First thing to do (work was interrupted mid-step)

The last command was cancelled before it ran. In order:

```
node target\tmp\edit40.js          (fixes 13 failing tool cases; if it throws
                                    "missing: ..." it was already applied, go on)
node scripts\dev\gen-tools.js
node scripts\dev\gen-skill.js
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\build.ps1 -Runtime
node scripts\dev\run-integration.js tools_e2e
```

`tools_e2e` ran once: **302 of 315 extra commands passed**. The 13 failures and
their fixes are in `edit40.js` (wrong expectations in the spec, one invalid CSS
selector in `handoff.detect_2fa`, `media.play` waiting forever on a video with
no source, `keyboard.key_down` removed). After it passes:

```
cargo fmt
cargo test --workspace --offline
cargo clippy --workspace --all-targets --offline -- -D warnings
cargo clippy --workspace --all-targets --offline --features agentgo-app/runtime -- -D warnings
node scripts\dev\run-integration.js            (12 test files, about 5 minutes)
node scripts\dev\coverage.js --write
node scripts\dev\audit.js
```

**The full suite has not been run since the changes of section 3.** The last
full green run (10 of 10) was before batch 1 of section 2. Since then each new
test file was run alone and passed, except `tools_e2e` (above). Expect small
breakage in older tests from: the account policy inversion, agent animations
(every click and fill now takes about 300 ms longer, typing arrives in several
`input` events), and `agent.js` listeners. `lockstep` may complain that
`integration.tools_e2e` is new; it exists.

## 2. Verified working (each has a passing integration test)

| Area | What | Test |
|---|---|---|
| Background window | never activated, never raised, taskbar-eligible, pages live while covered (`runtime/guard.rs`) | `presentation_e2e` |
| Agent input | own pointer and keyboard, also in tabs that are not on screen (tabs are stacked full-size, not hidden) | `presentation_e2e` |
| Tab close crash | focus emulation is switched off before a browser closes | `protocol_smoke` |
| Drag, clipboard, find, zoom, search records | `act.drag`, `clipboard.*` (private, not the Windows clipboard), `page.find`, `page.zoom`, `workflow.search_extract` | `daily_e2e` |
| Error page, session restore | `ui/error.html`; `--restore`, human launches restore by default | `daily_e2e` |
| Agent animations | pointer glide, click ripple, typing note, navigation banner; setting `agentAnimations` | `daily_e2e` |
| Laya interface | `decision.status/record/apply`, setting `layaEnabled` (operator only) | `daily_e2e`, unit tests |
| Account Center | list, add site, sign in (warning), unlink, **block/unblock** | `accounts_e2e` |
| Saved sign-ins | DPAPI vault, `credentials.fill` (agent never sees the password), save prompt after any sign-in, one-click key button | `accounts_e2e` |
| API connectors | OAuth + PKCE on loopback, `accounts.connect`, `connectors.list/call`, refresh, forged and replayed answers refused | `accounts_e2e` (mock provider) |
| Skill and MCP | `skills/agentgo/SKILL.md`, `references/commands.md` (generated), `references/laya.md`; MCP tools `browser_guide`, `browser_account` | `agentgo-mcp` unit tests |
| Packaging | `scripts\package.ps1` makes a portable folder and zip; ran two tests from a copy outside the repo | manual |
| Audit, bench | `scripts\dev\audit.js` (59 crates, 0 licence problems, 0 secrets), `scripts\dev\bench.js` (start 585 ms, observe 12 ms, click 395 ms, 726 MB; measured **before** animations) | scripts |

## 3. The owner's eight points (last message) and where each stands

| # | Asked | State |
|---|---|---|
| 1, 6 | Agent's own mouse and keyboard, animated, for every tool call | Done for click, hover, drag, fill, type and navigation. **Not animated:** scroll, select, check, key presses, waits. Add `page::show(&tab, "banner" or "pointTo", ...)` calls in `page.rs`. |
| 2 | Feels like a standard browser, all usual settings | **Not done.** Settings has: theme, accent, motion, control mode, agent tabs, Laya, animations, save sign-ins, notifications, downloads, hardware acceleration. Missing sections: On startup, Search engine, Privacy (clear cookies and site data, per-site permissions), Languages, Downloads folder, Default zoom, About. Missing UI: find bar (Ctrl+F), zoom indicator, bookmarks bar, context menu, print dialog, second window. |
| 3 | Verify the ~1000 tools | **Not true yet, measured:** the register has **1039** names. 97 implemented under their own name, 314 as tested aliases or page tools (pending the rerun in section 1), **628 not built**. See section 4. |
| 4 | Agent uses saved passwords without seeing them, one click | Done. |
| 5 | Save-password popup, also when the agent signs in; allow by default, Block button per account | Done. |
| 7 | Validate as ready to ship | **Not done.** See section 1 and section 5. |
| 8 | Skill + MCP entry teaching every tool; Laya plan preserved; Laya setting | Done. `references/commands.md` must be regenerated after every registry change (a unit test fails otherwise). |

## 4. Tools: what exists and what is left

How it works now:

- Core commands: `crates/agentgo-shell-bridge/src/methods.rs` (155 with tests).
- Extra commands: one line each in `scripts/dev/tools-spec.js` (name, target or
  page tool, description, test call, expected result). `gen-tools.js` writes
  `crates/agentgo-shell-bridge/src/extra.rs`; `tools_e2e.js` calls every line
  against `tests/fixtures/pages/tools.html`. Page tools are JavaScript in
  `crates/agentgo-cef/src/runtime/tools.js` (run in the isolated world).
- Every other register name is in `PLANNED` and answers
  `capability.unsupported` ("not built yet"). Nothing is silently unknown.
- `node scripts/dev/coverage.js --missing` lists the rest by domain;
  `--write` writes `docs/tool-coverage.md`.

To add a tool: add a line to the spec (and a function to `tools.js` if it is a
page tool), run `gen-tools.js`, `gen-skill.js`, build, run `tools_e2e`. The
owner agreed that names may be combined into one command when they do the
same thing, as long as each is tested.

Left to build, by kind (628 names), cheapest first:

1. **More aliases and page tools (about 120).** Domains 8 (observe.dom_snapshot,
   dom_subtree, iframe_content, cursor), 9 (ground.*), 10 (form.slider, color,
   datetime, rich_text, autocomplete), 20 (batch.*, recovery.*: most map to
   `workflow.batch` options or to an observe), 21 (collect.paginate, next_page,
   deduplicate, rank, filter, sort, limit), 23 (policy.*, handoff.*), 30 (api.*).
2. **DevTools-backed commands (about 110).** Add a third kind `cdp` to the spec
   and dispatcher. Network: block, offline/online, emulate latency and
   bandwidth, clear cache, headers, response bodies, HAR, websocket lists
   (`Network.*`, `Fetch.*`). Emulation: viewport.set_size, device scale,
   geolocation, orientation, locale, timezone, reduced motion, high contrast
   (`Emulation.*`). Touch: `Input.dispatchTouchEvent` for touch.start, move,
   end, swipe, pinch. Accessibility tree (`Accessibility.getFullAXTree`) for
   a11y.tree and observe.accessibility_tree. Storage clear
   (`Storage.clearDataForOrigin`). Tracing and profiling for debug.*.
   Init scripts and bindings for runtime.add_init_script, add_binding.
3. **Native browser features (about 150).** Tab duplicate, reopen closed, pin,
   mute, move, group, close others; window create, move, fullscreen, always on
   top; profile clone, export, import, proxy, locale; bookmarks edit, move,
   search; history search and delete; permissions list, grant, deny, reset
   with a UI; downloads open, show, verify (hash), quarantine; file.* scoped to
   the upload and download folders; print options; reader mode, view source,
   translate. `window.new` needs a second composition window: the code assumes
   one window (`state().window`).
4. **Whole subsystems, not started (about 250).** Extensions and user scripts
   (domain 18); native Windows UI Automation lane (26); workflow record and
   replay, skills management, memory (22); artifacts store with retention,
   redaction, search (25); metrics and benchmark suites including BrowserGym
   (29); agent task engine: plan, checkpoint, restore, parallel tabs, queue
   (19); the shell panels of domain 24 (command bar, sidebar, workflow editor,
   policy editor); update, rollback, sync, web apps; SDKs for Rust, TypeScript,
   Python (30); OCR, video recording, visual diff.
5. **Cannot be done in this engine; declare `unsupported` with the reason**
   instead of `planned`: `browser.engine_switch`, `browser.engine_matrix`,
   real hardware device access (`device.bluetooth`, `usb`, `serial`, `hid`,
   `nfc`), Widevine and proprietary codecs, Chrome Web Store, Google account
   sync, pen pressure. Add an `UNSUPPORTED` table next to `PLANNED`.

## 5. Known problems and unverified things

- **Real desktop:** the no-focus guard, the taskbar button and the "AgentGo"
  name are proven on the private desktop only. One real launch is still owed;
  ask the owner first. The guard lets the window come forward when the
  foreground belongs to `explorer.exe` or the Start, Search or Shell host;
  if Alt+Tab or the taskbar cannot bring the window up on some machine, that
  list in `guard.rs` is where to look.
- `human_active()` cannot be tested on the private desktop.
- Screenshots now contain the agent's pointer and notes. Hide the overlay
  during `observe.screenshot` if evidence must be clean.
- Benchmarks in `docs/benchmarks/performance.md` predate the animations. Run
  `bench.js` again; consider measuring with `agentAnimations` off as well.
- Unlink clears cookies of the listed hosts only (marked `ponytail:` in
  `accounts.rs`).
- Every open tab keeps rendering (marked `ponytail:` in `app.rs`).
- A built-in provider's API needs the person's own OAuth client id (entered in
  the Account Center). Real Google, GitHub and Yandex sign-in was never tried;
  only the mock provider was. GitHub's token endpoint requires a client secret.
- Cross-origin iframes are still not observed or acted in.
- Runtime resources are found by `ui\chrome.html` next to the executable, else
  by walking up to the repository.
- `dist\` and `target\` hold debug builds only. No release build was made, no
  installer, no code signing, no update mechanism.
- Stale documents: `docs/milestones.md`, `runtime-status.md`, `architecture.md`,
  `protocol.md`, `STATUS.md`, `HANDOFF.md`, `sonnethandoff.md`, the roadmap's
  "Current position". `docs/issues-log.md` needs rows for everything after D9
  (policy inversion, save prompt, animations, tool register count, Laya).
- Throwaway scripts in `target\`: `dbg-*.js` (quiet-run only), `tmp\edit*.js`.

## 6. Files added or heavily changed this session

`crates/agentgo-accounts/` (new crate), `crates/agentgo-shell-bridge/src/extra.rs`
(generated), `crates/agentgo-cef/src/runtime/{guard,accounts,connect,decision}.rs`,
`tools.js`, `agent.js`, `page.rs`, `dispatch.rs`, `cdp.rs`, `state.rs`, `app.rs`;
`ui/{chrome,tools}.{html,js}`, `ui/error.{html,js}`;
`tests/integration/{presentation,accounts,daily,tools}_e2e.js`, `harness.js`;
`tests/fixtures/pages/{account,login,daily,serp,tools}.html`;
`scripts/package.ps1`, `scripts/dev/{audit,bench,coverage,gen-skill,gen-tools,planned,tools-spec}.js`;
`skills/agentgo/`; `docs/{tool-register,licenses,HANDOFF-OPUS}.md`,
`docs/benchmarks/performance.md`.
