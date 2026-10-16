# Handoff: tools finished; next is the look and the installer (2026-10-15)

Read first: `docs/STATUS.md` (how to build and test), `docs/issues-log.md`
(every problem met, D1–D53), `docs/tool-coverage.md` (regenerate with
`node scripts/dev/coverage.js --write`).

## The owner's instructions (still in force)

- **Do not work on Laya / the decision model.** The owner starts that phase
  after the browser is finished.
- Never solve or bypass CAPTCHAs (detect, hand to the person).
- Log every issue and fix in `docs/issues-log.md`. No git commits unless asked.
- Integration tests run on the private desktop (`node scripts/dev/run-integration.js`),
  which fails if anything appears on the person's desktop.
- Next, in this order, when the owner says so:
  1. **Redesign the look**: "awwwards-level", motion-graphic feel for the
     chrome (`ui/chrome.*`), the panels (`ui/tools.*`), the New Tab page
     (`ui/index.html`, `ui/newtab.js`) and the error page. Keep every control
     wired to the same bridge commands; tests read some DOM ids
     (`#address`, `#tabs .tab`, `.tab-group`, `#agentOverlay`, `#personPrompt`,
     `.task-goal`, `.step.failed`, `[data-section=...]`, `.diff`,
     `[data-skill-toggle]`, `body.className`).
  2. **The installer** (`scripts/Install-AgentGo.ps1` today), same treatment.

## State now

All 1,039 register names: 1,026 working with tests, 13 impossible in the
engine with reasons, 0 missing. Product gaps closed: auto-update
(`browser.update_*`, `update.ps1`, `package.ps1` writes `update-manifest.json`)
and the code-signing path (`scripts/sign.ps1`, `package.ps1 -Sign`; needs the
owner's certificate).

New runtime modules (all in `crates/agentgo-cef/src/runtime/`):

| Module | What | Test |
|---|---|---|
| `tasks.rs` | agent task engine, batch transactions, recovery helpers, tab locks | `tasks_e2e` |
| `shell.rs` | `shell.*` panels, `omnibox.*` | `shell_e2e`, `windows_e2e` |
| `skills.rs` | local skills, `workflow.diff/verify/approve` | `shell_e2e` |
| `profiles.rs` | side profiles (own request contexts), `appearance.fonts` | `profiles_e2e` |
| `winmgr.rs` | secondary windows, detach/split/workspace, tab strip, frames | `windows_e2e` |
| `dialogs.rs` | held file pickers, HTTP sign-in, waits | `rest_e2e` |
| `inspect.rs` | OCR, visual grounding/diff, tracing, profiling, media, `runtime.*` | `rest_e2e` |
| `misc.rs` | IME, clipboard, drops, downloads, lists, user scripts, BrowserGym... | `rest_e2e` |
| `lifecycle.rs` | instances, restart, recover, budget, flags, auto-update | `rest_e2e` |
| `sdk.rs` | localhost HTTP + WebSocket transport, `sdk/` clients | `sdk_e2e` |

Every command that takes `ref` also takes `find: {by, ...}` (D49).

## How to add a tool

Unchanged: alias/page/cdp in `scripts/dev/tools-spec.js`; own commands in a
module routed at the top of `run_ungated` (`dispatch.rs`), registered with
`node scripts/dev/add-methods.js scripts/dev/methods/<file>.json`, then
`node scripts/dev/gen-tools.js`, build, test, `node scripts/dev/gen-skill.js`.
