# AgentGo Milestones — checklist and gates

Source: `outputs/agentgo_master_implementation_plan.md` (handoff). This
file is the living checklist; each milestone section lists its scope,
capability ids affected (from `schemas/capability-manifest.json`), tests,
and gate evidence. Update at every milestone start and completion.

## Milestone 0 — repository, toolchain, contracts — ✅ COMPLETE (2026-09-07)

Scope delivered:
- [x] Repository layout per plan section 6 (15 crates, ui/, docs/, schemas/, skills/, tests/, third_party/, scripts/, CI skeleton).
- [x] Pins: Rust 1.98.1 MSVC (`rust-toolchain.toml`), CEF `152.0.5+gb129680+chromium-152.0.7977.54` + cef-rs rev `83640144` (`Cargo.toml` + `scripts/pins.json` + `crates/agentgo-cef/src/pins.rs` in lockstep), CMake/Ninja/VS2026 requirements documented.
- [x] Git long-path guidance (`docs/development.md`) + deterministic bootstrap (`scripts/bootstrap.ps1`: pinned URL, size, SHA-1, SHA-256-once-recorded).
- [x] License (provisional dual MIT/Apache-2.0, ADR-0007) + NOTICE + third_party license inventory.
- [x] ADRs 0001-0008 (CEF, compositor, IPC, storage, secrets, Google search, license, Rust host).
- [x] Capability inventory converted to machine-readable manifest (`agentgo manifest generate`; owner/priority/risk/status/test id/skill ref per entry) with generation/validation tests.
- [x] Formatting/linting/unit tests/release+debug builds (`scripts/fmt.ps1`, `lint.ps1`, `test.ps1`, `build.ps1`) + Windows CI skeleton (`.github/workflows/ci.yml`).

Capability ids affected: `api.*` family (contract), `policy.*` (matrix),
`browser.version`/`browser.capabilities`/`browser.command_line`
(version/doctor reporting — contract only, handlers land with runtime),
`metrics.*` (none yet), all entries remain `planned`.

Tests: `agentgo-protocol` (envelope/framing/errors, 16 cases),
`agentgo-policy` (risk/matrix/scopes, 14), `agentgo-session`
(identity/leases, 7), `agentgo-storage` (paths/store, 9),
`agentgo-artifacts` (hash/redact, 7), `agentgo-shell-bridge`
(registry/origin, 10), `agentgo-cef` (pins/distro, 5),
`agentgo-observe`/`-locate`/`-actions`/`-workflows` (types, 10),
`agentgo-native-win` (honest unavailable, 2), `agentgo-mcp`
(mapping, 4), `agentgo-cli` (parse, 5), `agentgo-app` unit
(versions/manifest, 8) + integration `tests/lockstep.rs` (6) +
`scripts/test-protocol-conformance.ps1`.

Gate evidence (recorded 2026-09-07):
- `scripts/doctor.ps1` exit 0 (rustc 1.98.1 matches pin; cmake/ninja/git/python present; VS 2026 C++ tools detected; local layout writable; manifest in sync).
- `scripts/bootstrap.ps1 -Stage toolchain` + `-Stage fetch` idempotent pass; `-Stage cef` pinned download verified (size + sha1).
- `cargo test --workspace --locked` — all green.
- `cargo build --release` → `target/release/agentgo.exe` development binary; `agentgo version` reports all pinned versions.

## Milestone 1 — CEF and composition spike — ✅ COMPLETE (2026-09-08 runtime gate)

Scope:
- [x] Launch the normal Windows CEF runtime with the external message pump and clean shutdown.
- [x] Register `agentgo://shell` and load the lightweight trusted runtime shell from bundled resources.
- [x] Separate trusted shell and untrusted content BrowserViews; navigate the content surface to Google/example.com.
- [x] Add the first stdio protocol path and typed handler errors for the tested runtime slice.
- [x] Keep agentgo://shell bridge access limited to the dedicated main chrome frame.
- [x] Register the bundled asset factory in the isolated content context so the
  normal desktop landing page loads instead of producing `ERR_UNKNOWN_URL_SCHEME`.
- [x] Launch CEF with the official Windows sandbox bootstrap and `sandbox_info`;
  the packaged `bootstrap.exe` client DLL path loads the shell and landing
  resources and stays alive through the bounded startup smoke.
- [x] Load the full approved landing UI from bundled resources on normal
  desktop launch; the trusted chrome and landing surface remain separate.
- [x] Basic native tab creation, activation, close, bounded 4K resize, and
  viewport-local input are wired; broader touch/pen/IME/popup/DevTools parity
  remains correctly deferred to later capability milestones.
- [x] Decide BrowserView vs OSR/DirectComposition with measured results
  (ADR-0002 criteria); BrowserView remains the selected composition.
- [x] Renderer termination/unresponsive state is separated from browser state;
  bounded redacted local crash metadata is available through
  `browser.crash_report`.

Capability ids affected: `browser.launch`, `browser.attach` (partial),
`browser.shutdown`, `browser.health`, `browser.version`,
`browser.capabilities`, `browser.crash_report`, `tab.create/close/
activate/list/status/navigate/reload/stop/back/forward`,
`window.*` (view composition subset), `navigate.url`, `navigate.search`
(Google), `wait.load`, `shell.*` (bridge wiring),
`download.start/list/progress/pause/resume/cancel` (CEF download handler
proof).

Gate evidence: ten representative public sites rendered and settled in the
BrowserView, each accepted native pointer input, and the protocol smoke
exercised 3840×2160 resize plus pointer input before restoring the window.
The shell is a separate topmost sibling BrowserView, and content remains
outside the trusted bridge origin. Exact commands and results are in
`docs/runtime-status.md` and ADR-0002.

## Milestone 2 — profiles, storage, product shell — ✅ COMPLETE (2026-09-08 foundation gate)

Delivered in the current foundation slice:
- [x] Canonical local layout is created at `%LOCALAPPDATA%\AgentGo` (or the
  explicit test override `AGENTGO_DATA_ROOT`).
- [x] Settings and bounded history survive process restarts through atomic,
  versioned records; credential-shaped settings are rejected.
- [x] Default desktop launch opens the approved landing UI; agent stdio launch
  remains Google-first.
- [x] Google-only search resolution, site metadata, and explicit history-clear
  confirmation are available over stdio and the dedicated chrome bridge.
- [x] Native minimize/maximize-or-restore controls are wired through the
  protocol and trusted chrome bridge.
- [x] Profile metadata catalog, launch-time profile selection, and scoped CEF
  download lifecycle foundation are available; downloads remain disabled by
  default and are never executed. Explicit start/progress/pause/resume/cancel
  controls are now wired to CEF's native download callbacks.
- [x] Content permission requests default-deny and native file selection is
  handed to CEF's Windows picker with a safe audit event.
- [x] Native window resize accepts bounded dimensions and relayouts the shell
  and content surfaces through the existing window delegate.
- [x] Basic multi-tab lifecycle creates, activates, and closes attached native
  BrowserViews in the shared isolated content context.
- [x] SQLite WAL state store now owns URL-only history and establishes tables
  for bookmarks, workflow checkpoints, and audit events.
- [x] Each run publishes an atomic local instance endpoint lifecycle record
  with an opaque id, pid, protocol, transport, selected profile, and clean
  stop status; secrets and cookies never enter the record.
- [x] Registry, manifest, skill, docs, and integration smoke agree on the
  reviewed implementation slice.
- [x] Approved landing typography is bundled locally with OFL notices; the
  landing surface no longer depends on Google Fonts at runtime.
- [x] Landing bookmarks and shortcut tiles navigate to their real HTTPS
  destinations from the isolated content surface.
- [x] The bounded native History/Downloads/Settings overlay is restacked after
  new content tabs, keeping browser utilities available above every page.

M2 boundary note:
- Authenticated named-pipe attachment is intentionally a Milestone 3
  transport; M2 records contain only non-secret lifecycle metadata.
- The M2 foundation slice above is implemented and smoke-tested. The broader
  product-shell capability inventory remains `planned` until its native
  handlers and evidence are added; do not infer support from prototype UI.

## Milestone 3 — authenticated local control — PLANNED

- [ ] Feature negotiation and strict protocol gateway.
- [ ] Windows named-pipe transport with current-user ACLs, bootstrap token,
  session identity, leases, attach/detach, and no TCP listener.
- [ ] SDK/MCP adapters generated from the same protocol contracts.

## Milestone 4 — human-equivalent input and page actions — PLANNED

- [x] Basic viewport-local pointer move/click/scroll and named keyboard
  type/press dispatch are implemented and covered by protocol smoke.
- [ ] Keyboard, mouse, wheel, touch/pen, clipboard, focus, form, upload, and
  action verification paths with policy under the adapter boundary.

## Milestone 5 — semantic observation and grounding — PLANNED

- [ ] Outline/actionable/content/form/change-set views, DOM/AX/layout
  extraction, visual evidence, stable element references, and stale-target
  recovery with bounded output budgets.

## Milestone 6 — one-call workflows and recovery — PLANNED

- [ ] Goal execution, deterministic batches, postconditions, idempotency,
  checkpointing, retry/recovery, research/extraction, and resumable workflows.

## Milestone 7 — approvals, profiles, and human handoff — PLANNED

- [ ] Full/approval modes, leases, audit trail, pause/resume/stop semantics,
  credential/login/2FA/CAPTCHA handoff, and permanent approval gates for
  purchases, payments, transfers, and other money movement.

## Milestone 8 — files, native Windows, media, and evidence — PLANNED

- [ ] Downloads, file/print dialogs, notifications, media/device permissions,
  screenshots/PDF/HTML artifacts, native owned-window control, and Windows
  sandbox packaging.

## Milestone 9 — skills, replay, adapters, and low-token context — PLANNED

- [ ] Versioned AgentGo skill delivery, compact observation diffs, replay,
  memory/domain adapters, artifacts, Rust/TypeScript/Python SDKs, and MCP.

## Milestone 10 — extensions, diagnostics, and product hardening — PLANNED

- [ ] Extensions/plugins, crash separation/restart, diagnostics, budgets,
  accessibility, settings UI wiring, and measured BrowserView/compositor choice.

## Milestone 11 — evaluation and release gate — PLANNED

- [ ] Conformance, security, performance, accessibility, and browser benchmark
  suites; ten modern-site render/control passes; reproducible packaging,
  release evidence, and a final capability-manifest audit.
