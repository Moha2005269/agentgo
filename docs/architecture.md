# AgentGo Architecture

Status: Milestones 1-2 runtime foundation. This document records the target
architecture from the master plan and the honest current state of each
piece. It is updated at every milestone.

## Target shape

```text
External agent (Codex / Claude Code / OpenCode / any local client)
          |
          | stdio JSON or authenticated Windows named pipe
          v
Protocol gateway + capability negotiation + session authentication
          |
          v
Workflow coordinator ----> policy/approval engine ----> audit log
    |          |                    |                       |
    |          |                    +--> human handoff UI   +--> local artifacts
    |          v
    |     semantic state reducer <---- CEF/CDP/AX/layout/network
    |          |
    +--> locator + action + wait + verification + bounded recovery
               |
               +--> CEF browser/page input
               +--> native Windows lane for owned windows/dialogs

Win32/CEF host
    +-- trusted AgentGo shell (`agentgo://shell`, bundled approved UI)
    +-- isolated page browsers/tabs/profiles
    +-- downloads, permissions, dialogs, storage, media, DevTools hooks
    +-- crash recovery, process sandboxing, resource budgets
```

## Canonical entities (stable opaque ids; raw pointers never cross the boundary)

`instance_id`, `client_id`, `profile_id`, `workspace_id`, `window_id`,
`tab_id`, `frame_id`, `document_generation`, `element_ref`,
`workflow_id`, `step_id`, `approval_id`, `handoff_id`, `artifact_id`,
`trace_id`, `lease_id`.

## Repository layout

See the master plan section 6; the crates live under `crates/`:

| Crate | Role | Milestone 0 state |
|---|---|---|
| agentgo-app | executable + composition root (version/doctor/manifest) | implemented + tested |
| agentgo-cli | CLI parsing/dispatch vocabulary | implemented + tested |
| agentgo-protocol | envelopes, framing, error codes, version pins | implemented + tested |
| agentgo-policy | risk classification, approval matrix, scopes | implemented + tested |
| agentgo-session | instance identity, secrets, leases | implemented + tested |
| agentgo-storage | local layout + atomic versioned settings/history store | implemented + tested |
| agentgo-artifacts | hashing + redaction | implemented + tested |
| agentgo-shell-bridge | shell method registry + origin trust boundary | implemented + tested |
| agentgo-cef | CEF pins + distribution verifier | implemented + tested (runtime wiring: Milestone 1) |
| agentgo-observe | view taxonomy, outline, element refs | types + tests |
| agentgo-locate | locator strategies | types + tests |
| agentgo-actions | typed actions + risk | types + tests |
| agentgo-workflows | batches, postconditions, idempotency | types + tests |
| agentgo-native-win | native lane (honest unavailable errors) | contract + tests |
| agentgo-mcp | MCP naming/mapping validation | contract + tests |

## Trust boundaries

1. **Trusted shell vs untrusted pages.** The native bridge exists only
   for the exact `agentgo://shell` origin/main frame (tested predicate in
   `agentgo-shell-bridge::origin`). Pages run in separate request
   contexts/renderers and can never call the bridge.
2. **Policy below every adapter.** CLI, MCP, SDK, shell, replay, and
   page scripts all pass through the same decision matrix
   (`agentgo-policy::mode`); money actions always stop; CAPTCHA always
   hands off; certificate bypass is always denied.
3. **Page content is untrusted input.** It can never grant permissions,
   install skills, modify policy, or issue native commands.
4. **Local data only.** No cloud sync/telemetry by default; secrets via
   DPAPI (ADR-0005); redaction before persistence.

## Current honest state (Milestones 1-2 runtime foundation)

- The normal Windows CEF runtime uses CEF's built-in message loop and hardware
  compositor. Stdio/smoke harnesses retain the bounded external-pump path.
  The top-level Window uses a vertical CEF BoxLayout: fixed trusted chrome is
  the first child and the visible isolated content BrowserView is the flex
  child below it.
- The current stdio path is an initial local agent transport, not the final
  authenticated named-pipe gateway.
- Normal desktop launch composes the small trusted `ui/chrome.html` chrome
  above the full approved `ui/index.html` landing surface. Agent stdio
  launches keep the content start page on Google. The landing document is
  served through the isolated content view and is intentionally unable to
  call the native bridge; only the main `chrome.html` frame can do so.
- Content BrowserViews are direct Window siblings rather than manually offset
  descendants of a nested panel. CEF therefore assigns each visible site a
  native viewport beginning exactly after the 104-DIP chrome, including its
  own top-of-page controls. The local landing page uses an explicit embedded
  layout that removes only its duplicate prototype rows.
- Trusted toolbar utilities render in the dedicated `ui/tools.html`
  BrowserView. Private `ui.open_tool` and `ui.close_tool` bridge commands
  show only a tightly bounded 440px popover or 720px drawer. The active content
  tab remains visible and unchanged outside that opaque native rectangle.
- Settings and bounded history are persisted under the canonical local
  `%LOCALAPPDATA%\AgentGo` layout (or `AGENTGO_DATA_ROOT` for tests) using the
  atomic versioned store. No generic settings endpoint accepts secrets.
- Every run writes an atomic non-secret instance endpoint lifecycle record
  under `instances\\<instance_id>\\endpoint.json`; the record is suitable for
  local diagnostics and future authenticated attachment, while named-pipe
  authentication itself remains Milestone 3.
- Renderer termination and unresponsive callbacks are now separated from the
  browser lifecycle, with bounded redacted local diagnostics. Permission
  requests default-deny and native file dialogs remain on CEF's Windows
  handoff path.
- The official Windows sandbox bootstrap path is packaged and verified with
  the Rust `RunWinMain` client DLL entry. Direct invocation of the sandbox
  feature still fails closed by design; the official bootstrap package is the
  only supported sandbox entry path for this foundation.
- The manifest and shell registry now promote only the tested navigation,
  health/version, settings, history, search-resolution, site-info, clean
  shutdown, native window controls including bounded resize, basic native
  multi-tab lifecycle, pointer/keyboard dispatch, crash metadata, default-deny
  permissions, native picker handoff, and download
  start/progress/pause/resume/cancel metadata slice. The
  remaining inventory stays `planned` and returns `capability.unsupported`.
- The approved UI ships verbatim at `ui/index.html`; its prototype
  behaviors (mock agents, timers, fake downloads) are documented
  obligations for Milestones 2-3, not product features, and the lockstep
  tests pin the migration duties.

See `docs/runtime-status.md` for the exact implemented method slice,
commands, evidence, limitations, and next work order.
