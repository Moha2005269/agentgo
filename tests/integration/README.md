# Integration tests

Cross-cutting integration tests live in the crates' `tests/` directories
(e.g. `crates/agentgo-app/tests/lockstep.rs`). This directory holds
harnesses that need a built product:

- `smoke_m1.md` — Milestone 1 spike scenario (added with the spike):
  launch bundled app, load `agentgo://shell`, verify the trusted origin
  gate, navigate the content surface to Google, drive the fixture page,
  capture evidence artifacts, exit cleanly.
- `protocol_smoke.js` — stdio protocol smoke: strict JSON stdout,
  health/version/capabilities, typed unknown/planned/invalid errors,
  content navigation/status, profiles/settings/history/search/site info,
  multi-tab create/activate/close, native pointer/keyboard dispatch, download
  start/progress/pause/resume/cancel/lifecycle/gates, crash-report and
  permission-policy health fields, window controls, bounded 4K resize/input,
  and clean shutdown.
- `site_render_smoke.js` — opt-in ten-site BrowserView gate: navigates public
  modern sites one at a time, waits for settled status, dispatches a native
  pointer move, and records the final URL before clean shutdown. It never
  submits forms or uses authenticated accounts.

Run the whole suite with `node scripts\dev\run-integration.js`.
