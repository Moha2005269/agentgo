# AgentGo Browser — Master Implementation Plan

Version: 1.0 planning handoff  
Date: 2026-09-07  
Target: Windows 11 x64 first  
Status: plan only; implementation has not started

## 1. Mission

Build AgentGo into the most advanced agent-first browser: a real Chromium/CEF browser designed primarily for external agents, not a consumer browser with automation added afterward. Its primary operator is the agent; the human-facing UI exists for visibility, policy, approvals, handoff, settings, and immediate manual takeover. The entire product surface must be controllable by any external coding agent with the same capabilities. AgentGo is not an AI provider and must never require a model-provider API key. Codex, Claude Code, OpenCode, and other callers all use one local, open, versioned control protocol.

After non-bypassable safety requirements, product priorities are:

1. Agent task success and complete browser-control coverage.
2. Minimal token usage and minimal external-agent reasoning turns.
3. Easy, discoverable, vendor-neutral use by any agent host.
4. Speed, deterministic execution, recovery, and parallel isolation.
5. Human supervision quality and ordinary manual-browser usability.

Do not spend effort imitating every consumer-browser feature before the agent-control, compact-observation, workflow, safety, and skill foundations are excellent.

AgentGo must combine:

- normal browser quality and human control;
- complete browser and scoped Windows input control;
- compact semantic observation that minimizes model tokens;
- deterministic batch execution, verification, recovery, replay, and structured extraction;
- visible user control with Pause and destructive Stop Agent actions;
- strict safety gates for purchases and human handoff for CAPTCHA;
- local profiles, artifacts, audit history, settings, and credentials;
- the approved AgentGo HTML interface.

The exhaustive function inventory remains the command-level source of truth:

- `outputs/agentgo_capability_inventory.md`
- `outputs/agent_browser_research.md`
- UI source: `work/agentgo-ui/index.html`
- research archive: `work/agent-browser-sources/`

Every function in the capability inventory must eventually have an implementation, typed schema, risk classification, skill entry, automated test, example, and capability-discovery record. Never mark a milestone complete by replacing behavior with a toast, timer, hard-coded result, or visual simulation.

## 2. Non-negotiable product decisions

1. CEF/Chromium is the active engine. Servo, Verso, Zervo, and Ladybird are research only.
2. Windows x64 is the only first-release platform.
3. The approved HTML/CSS/JS in `work/agentgo-ui/index.html` is the browser shell and visual baseline. Preserve its identity, logo, typography, graphite palette, orange accent, wing/halo treatment, transitions, spacing, and layout unless a native integration constraint is documented.
4. AgentGo is agent-first. Human usability remains required, but architecture and prioritization are optimized for agent reliability, control depth, token efficiency, observability, and safe autonomy.
5. Google is the default and primary search provider. Remove “AgentGo Search” from the product. A plain query navigates the active tab to `https://www.google.com/search?q=...`. AgentGo does not proxy search and does not provide a hosted search backend.
6. The app makes no hidden model calls and contains no required OpenAI, Anthropic, Google AI, or other model API.
7. Agents communicate through a local protocol, not a hosted web service. Stdio JSON and a Windows named pipe are the primary transports. MCP, TypeScript, Python, and Rust adapters are thin mappings over the same schema.
8. Every agent gets an isolated process/session/profile lease with a cryptographically random instance ID and connection secret. Use one signed executable launched multiple times; do not create mutated executable copies.
9. A user may launch AgentGo first and then give an external agent the displayed connection code.
10. The agent can control the whole AgentGo product: tabs, windows, browser chrome, pages, settings, downloads, dialogs, profiles, files, and scoped Windows-native dialogs.
11. Full-Access Mode should not interrupt ordinary browsing, forms, uploads, downloads, messages, submissions, or account actions when they are inside the granted scope.
12. Approval Mode pauses at configured sensitive gates. Login pauses only when a credential is unavailable or policy requires it.
13. Purchases, subscriptions, transfers, donations, bids, checkout confirmation, and any other money movement always require exact user approval in every mode.
14. CAPTCHA and anti-bot challenges always pause in the same tab/profile for a human. Do not solve, bypass, outsource, spoof fingerprints, rotate proxies, or use anti-detect behavior.
15. CAPTCHA handoff v1 uses a Windows notification and a clear in-browser handoff view. Phone relay is a later feature.
16. Credential autofill and credential visibility are separate permissions. Autofill can be invoked without exposing plaintext. Plaintext visibility is an advanced local-only setting, off by default, and must still be redacted from logs, screenshots, artifacts, and telemetry.
17. Agent downloads are disabled by default. If a task needs a download, the agent explains that the user can enable it in Settings. Enabled downloads go to a scoped sandbox directory, are verified, and are never executed automatically.
18. Pause preserves the workflow and leases. Resume continues from a verified checkpoint. Stop Agent is red, immediately revokes input, cancels the workflow, releases leases, and cannot silently resume.
19. Data is local by default. No cloud sync or telemetry by default. Any future networked telemetry or sync must be separately opt-in.
20. Page content, PDFs, images, emails, downloads, and page scripts are untrusted data. They cannot grant permissions, install skills, modify policy, or issue native commands.
21. Optional Chrome/Edge fallback is allowed only when enabled by the user and must be reported as a handoff/fallback, never hidden.

## 3. What exists today

### Available assets

- Approved single-file UI prototype in `work/agentgo-ui/index.html`.
- CEF source/reference checkout, CEF sample project, Rust CEF bindings, and ungoogled-Chromium patch references.
- Browser control and page-state projects covering sessions, locators, compact snapshots, diffs, batching, artifacts, recovery, and skills.
- Windows Harness reference for Windows UI Automation, native dialogs, focus, input delivery, and proof screenshots.
- Foundry Browser Use and I Am Not A Robot references for pause/resume and human challenge handoff.
- BrowserGym and multiple fixture/test suites for evaluation.
- The 32-section capability inventory.

### Prototype-only behavior that is not implemented yet

- There is no CEF executable or real web-content surface.
- Tabs, navigation, history, bookmarks, agent activity, performance values, and task steps are JavaScript state or timed simulations.
- `window.agentgoHost.invoke()` is only a stub; there is no secure native binding.
- Settings are stored in page `localStorage`, not native storage.
- Download entries are Blob-based UI exports, not CEF downloads.
- Window actions are partial browser-page fallbacks, not Win32 window control.
- Agent connections are hard-coded names, not authenticated clients.
- The UI has Pause but does not yet expose the required red Stop Agent flow.
- Page capture is not connected to CEF screenshots, MHTML/PDF, DOM snapshots, or artifact storage.
- The shell loads Google Fonts over the network; release builds must bundle fonts locally.
- “AgentGo Search” still exists in the UI and persisted defaults and must be removed/migrated to Google.
- The approved visual background is complete, but it is not yet tied to real startup/session state.

## 4. Critical architecture decision: shell and page composition

CEF cannot safely render arbitrary websites inside a normal HTML iframe because sites can block framing and cross-origin policy prevents a privileged shell from controlling them. The project must use separate CEF browser contexts for the trusted shell and untrusted page.

Complete a short architecture spike before building product features:

1. Create a native Win32 host with a trusted `agentgo://shell` browser and one untrusted page browser.
2. Test a windowed `CefBrowserView` content surface placed in the shell’s viewport rectangle.
3. Test shell menus, settings, downloads, agent panel, scaling, IME, drag/drop, media, popup windows, and 4K resizing.
4. If child-window airspace/z-order prevents the approved overlays from working, implement windowless CEF rendering for the content surface and composite it with DirectComposition/D3D11.
5. Do not continue to broad feature work until the chosen compositor sustains smooth resize/scroll/input and keeps the privileged shell visually and logically separate from pages.

Recommended first route: use `CefBrowserView` for the P0 proof because it is simpler and hardware accelerated. Internal pages such as New Tab, Settings, History, Downloads, and Agent Activity can hide the external content view and use the full shell. If overlay behavior cannot preserve the UI, move only the page surface to OSR/DirectComposition rather than rewriting the shell.

Security invariants:

- The privileged native bridge is available only to the exact `agentgo://shell` origin and main frame.
- Normal websites use a separate request context/renderer and can never call the shell bridge.
- The shell applies a strict CSP and loads bundled local assets only.
- The page surface may crash or navigate without taking down the shell or policy engine.

## 5. Target architecture

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
    +-- trusted AgentGo shell (`agentgo://shell`)
    +-- isolated page browsers/tabs/profiles
    +-- downloads, permissions, dialogs, storage, media, DevTools hooks
    +-- crash recovery, process sandboxing, resource budgets
```

### Canonical entities

Use stable opaque IDs and never expose raw pointers/handles to agents:

- `instance_id`
- `client_id`
- `profile_id`
- `workspace_id`
- `window_id`
- `tab_id`
- `frame_id`
- `document_generation`
- `element_ref`
- `workflow_id`
- `step_id`
- `approval_id`
- `handoff_id`
- `artifact_id`
- `trace_id`
- `lease_id`

## 6. Recommended repository layout

Create a new repository; do not develop inside any downloaded reference project.

```text
work/agentgo/
  Cargo.toml
  rust-toolchain.toml
  LICENSE
  NOTICE.md
  SECURITY.md
  CONTRIBUTING.md
  docs/
    adr/
    architecture.md
    protocol.md
    policy.md
    threat-model.md
    compatibility.md
  schemas/
    protocol.schema.json
    capability-manifest.json
    policy.schema.json
    skill.schema.json
  crates/
    agentgo-app/             # executable and composition root
    agentgo-cef/             # CEF runtime adapter
    agentgo-shell-bridge/    # trusted shell/native messages
    agentgo-protocol/        # messages, framing, versioning, generated schemas
    agentgo-session/         # instances, profiles, leases, checkpoints
    agentgo-observe/         # DOM/AX/layout/visual/network reducer
    agentgo-locate/          # semantic and hybrid target resolution
    agentgo-actions/         # pointer/keyboard/forms/browser actions
    agentgo-workflows/       # batch executor, waits, verification, recovery
    agentgo-policy/          # risk classification, approvals, scopes
    agentgo-artifacts/       # files, evidence, redaction, hashes, retention
    agentgo-storage/         # SQLite settings/history/bookmarks/audit metadata
    agentgo-native-win/      # Win32/UIA/dialogs/notifications/Defender hooks
    agentgo-cli/             # universal command-line client
    agentgo-mcp/             # thin optional MCP adapter
  ui/
    index.html
    assets/fonts/
    assets/icons/
  skills/
    agentgo/SKILL.md
    agentgo/references/
  sdk/
    rust/
    typescript/
    python/
  tests/
    fixtures/
    protocol/
    integration/
    policy/
    security/
    browsergym/
    native-windows/
  third_party/
    README.md
    licenses/
```

Use Rust for the policy, protocol, state, workflow, storage, artifacts, and Windows-control core. Start with `cef-rs` pinned to an exact revision. If a required CEF feature is missing or unsafe in the bindings, add a very small official-API C++ shim behind a documented C ABI; do not spread C++ across the core.

## 7. Local control protocol

### Transports

P0 transports:

1. Newline-delimited JSON over stdio for an agent that launches AgentGo.
2. Windows named pipe for attaching to an already-running instance.

Do not expose a TCP port by default. If a loopback transport is added later, it is disabled by default and uses the same authentication and schemas.

Named-pipe requirements:

- Pipe ACL restricted to the current Windows user SID.
- Random 256-bit instance secret and unique instance ID.
- One-time bootstrap token exchanged for a short-lived client session.
- Protocol/version negotiation before commands.
- Client identity, capability grant, and lease recorded in the local audit log.
- Heartbeats and disconnect cleanup.
- Maximum message size and bounded queues.
- Artifact bytes stay on disk; the protocol returns handles and metadata instead of huge base64 blobs.

### Message shape

Use a small JSON-RPC-like envelope:

```json
{"v":1,"id":"req_01","method":"tab.navigate","params":{"tab_id":"tab_01","url":"https://www.google.com/search?q=agentic+browser"},"idempotency_key":"..."}
```

Responses contain `ok`, typed `result` or typed `error`, `trace_id`, `state_revision`, and recovery hints. Events contain a monotonic sequence number and include workflow progress, tab changes, downloads, dialogs, approvals, challenge handoff, crashes, and artifact completion.

### Three execution levels

The “one call” requirement must not smuggle an AI model into AgentGo:

1. Low-level commands: exact browser/page/native actions.
2. Deterministic workflows: one call executes a typed recipe such as search-and-extract, fill-and-review, paginate-and-collect, download-and-verify, or replay. No model is needed.
3. Open-ended agent session: the external agent registers as the planner. AgentGo performs observation reduction, batching, waiting, verification, and recovery, then emits `planner.decision_required` only when a semantic decision is genuinely needed. To the user this remains one running task, but planning still belongs to the external agent.

Do not implement `agent.run(goal: string)` as if the browser can understand arbitrary goals without a planner. Either require a typed workflow/plan or use the registered external-planner callback.

## 8. Shell integration contract

Replace the prototype’s free-form `window.agentgoHost.invoke(name, detail)` with a versioned, promise-based native RPC available only to the shell:

```js
await window.agentgo.request({ id, method, params })
window.agentgo.subscribe(event => render(event))
```

Rules:

- Native state is authoritative. UI state is a projection.
- The UI may optimistically animate safe cosmetic state, but it must reconcile with native results.
- Remove fake timers, hard-coded agents, fake navigation, fake latency/memory, and synthetic task progress when the corresponding native stream is connected.
- Replace localStorage authority with native settings/history/bookmarks/profile storage. LocalStorage may cache non-sensitive presentation state only.
- Every clickable control in the approved UI maps to a typed protocol method.
- Unsupported controls must be visibly disabled with an honest reason; never show a success toast for an unimplemented action.
- Bundle fonts and icons locally.
- Add a persistent red Stop Agent control beside Pause whenever a workflow owns browser input.
- The shell must render: active agent, goal, current high-level step, paused/handoff state, exact approval request, downloads, artifacts, trace/evidence, and errors.

Immediate UI migration tasks:

1. Remove “AgentGo Search” option and default; migrate existing stored value to `Google`.
2. Route omnibox queries to Google and URLs directly to the active CEF tab.
3. Replace mock tabs with native tab events and stable tab IDs.
4. Replace fake history/bookmarks/downloads/settings with native stores.
5. Replace static agent buttons with authenticated connected clients.
6. Wire Pause, Resume, Step, Stop Agent, agent-enable, approvals, and CAPTCHA handoff.
7. Turn History, Downloads, Settings, Site Info, Capture, New Window, Sandbox Window, and all window controls into real host actions.
8. Keep New Tab as a trusted internal shell page; real sites render in the untrusted content surface.

## 9. Storage and filesystem layout

Recommended local layout:

```text
%LOCALAPPDATA%\AgentGo\
  config\settings.json
  data\agentgo.db
  profiles\<profile_id>\
  instances\<instance_id>\endpoint.json
  artifacts\<workflow_id>\
  downloads\<profile_id>\
  traces\<workflow_id>\
  crashes\
  logs\
```

- Use SQLite WAL for settings, history, bookmarks, workflow metadata, audit indexes, capabilities, and resumable checkpoints.
- Use per-profile CEF request contexts/cache directories.
- Use Windows Credential Manager or DPAPI-protected vault entries for secrets; never place plaintext credentials in SQLite.
- Use atomic writes and schema migrations.
- Redact secrets before logging or artifact persistence.
- Keep downloads/artifact access within explicitly granted directories.
- Apply retention policies and provide complete local deletion/export.

## 10. Policy and approval matrix

Evaluate policy below every adapter so CLI, MCP, SDK, UI, replay, and page scripts cannot disagree.

| Action | Approval Mode | Full-Access Mode |
|---|---|---|
| Read/navigate/search/scroll | proceed in scope | proceed in scope |
| Type ordinary form data | configurable; default proceed | proceed in scope |
| Login with stored vault credential | proceed if enabled | proceed if enabled |
| Missing credential or 2FA | human handoff | human handoff |
| CAPTCHA/challenge | always human handoff | always human handoff |
| External message/final submission | default approval | proceed if scoped |
| Account/security change | approval | proceed only if explicitly scoped |
| Delete/revoke/destructive cloud action | approval | proceed only if explicitly scoped and reversible policy allows |
| Upload local file | require file-scope grant | proceed inside file scope |
| Download | only if user enabled | only if user enabled |
| Execute downloaded file | separate explicit approval | separate explicit approval |
| Purchase/payment/transfer/subscription/bid/donation | always approval | always approval |
| Certificate error bypass | deny | deny |

Approval records bind the exact origin, account, action, target, fields/data summary, amount/currency where applicable, expiry, workflow, and idempotency key. Any meaningful change invalidates approval.

Money-action detection must be defense in depth: semantic page text, form fields, page identity, URL, merchant/amount summary, request metadata where permitted, and known checkout transitions. If uncertain at the final side-effect boundary, fail closed and ask.

## 11. Observation and token-efficiency design

Build one internal page model from CEF renderer data and DevTools/Accessibility domains. Provide views instead of repeatedly dumping everything:

- outline: URL, title, state, major regions, headings, dialogs, frames;
- actionables: roles, names, labels, states, bounds, allowed actions, stable references;
- content: cleaned article/list/table/result records;
- forms: fields, requirements, values, validation, submit targets;
- change set: only revisions since a supplied baseline;
- visual: screenshot or targeted crop only when semantics are insufficient;
- raw HTML/DOM: bounded and explicitly requested;
- network-derived structured data: only when permitted and redacted.

Stable element references should contain an opaque ID backed by frame ID, document generation, backend DOM/AX identity, semantic fingerprint, and last-known bounds. Validate immediately before action. If stale, re-resolve by semantic fingerprint, report confidence, and stop rather than guessing when ambiguous.

Token controls:

- intent-filter every observation;
- return bounded collections with pagination handles;
- baseline plus deltas after actions;
- store screenshots, HTML, PDFs, network bodies, and traces as artifacts;
- use targeted crops rather than full screenshots;
- batch deterministic actions with automatic waits;
- deduplicate pagination/infinite-scroll results in-process;
- cache stable extraction and invalidate by fingerprint;
- compile successful domain workflows into parameterized skills;
- measure success per token, model turns, actions, time, and recovery count.

## 12. Action, synchronization, and recovery design

Use CEF-native input for web content first. Use DOM/semantic actions when they preserve normal page events, and fall back to coordinate input only with a fresh hit test. Use Windows UI Automation/SendInput only for AgentGo-owned windows and explicitly selected native dialogs.

Every action pipeline:

1. Resolve target and frame.
2. Validate document generation and target state.
3. Classify risk and check policy.
4. Acquire tab/input lease.
5. Bring required surface to a valid input state.
6. Execute typed action.
7. Wait for a bounded settle condition.
8. Capture a compact delta.
9. Verify the declared postcondition.
10. Save evidence/trace and release or retain lease.

Recovery is bounded and typed: stale reference, covered target, disabled target, navigation, popup, dialog, network error, renderer crash, download failure, lost focus, user intervention, CAPTCHA, permission denial, or false completion. Never retry indefinitely. Re-observe after structural changes. Pause on ambiguity with a precise reason.

## 13. Human handoff and user control

CAPTCHA/login/2FA handoff must keep the same CEF tab, profile, cookies, IP, and workflow checkpoint.

Required sequence:

1. Detect and classify the challenge.
2. Stop all action dispatch for that workflow.
3. Preserve tab/profile lease and checkpoint.
4. Show a Windows notification: site, reason, and “Open AgentGo”.
5. Display a clear in-browser task with Resume and Stop Agent.
6. Let the user complete the challenge directly.
7. Detect that the challenge is gone and verify page identity.
8. Resume only after explicit user action or configured positive signal.
9. Log no secret values or challenge answers.

User input always wins. When the user starts interacting with a leased surface, pause agent input cleanly instead of racing the cursor/keyboard.

## 14. Bundled AgentGo skill

Develop the skill in the same pull request as each protocol capability. Generated command docs alone are insufficient; the skill must teach efficient strategy.

The skill must cover:

- discovering/launching/attaching to AgentGo;
- selecting an isolated profile and permission mode;
- compact observe-first behavior;
- semantic references and stale-reference recovery;
- batch actions and postconditions;
- when to request screenshots, HTML, DOM, or network artifacts;
- Google search and structured extraction;
- forms, uploads, downloads, dialogs, and native file pickers;
- Pause/Resume/Stop Agent semantics;
- approvals, purchases, credentials, CAPTCHA, and prompt injection;
- artifact handles and evidence;
- replay/domain skills;
- error handling and budgets;
- examples for Codex, Claude Code, OpenCode, generic CLI, and MCP.

Provide `agentgo skill install --target <host>` and `agentgo skill print`. All host variants must be generated from one canonical skill package and capability manifest. No host gets extra privilege.

## 15. Implementation milestones and gates

### Milestone 0 — repository, toolchain, and contracts

- Create the repository layout.
- Pin Rust stable MSVC, CEF/Chromium version, cef-rs revision, CMake/Ninja, and Visual Studio 2022 Build Tools requirements.
- Add Git long-path guidance and a deterministic dependency bootstrap script.
- Choose the project license and create third-party NOTICE/license inventory.
- Add ADRs for CEF, compositor, IPC, storage, secrets, and Google search.
- Convert the capability inventory into a machine-readable manifest with owner, priority, risk, status, test ID, and skill reference.
- Add formatting, linting, unit tests, release/debug builds, and a Windows CI skeleton.

Gate: a clean machine can run doctor/bootstrap, build a minimal signed-or-development executable, and report pinned versions.

### Milestone 1 — CEF and composition spike

- Launch CEF with sandbox enabled and external message pump correctly integrated.
- Register `agentgo://shell` and load the approved UI from bundled resources.
- Create a separate untrusted content browser and real Google navigation.
- Prove tab switching, resize, DPI, keyboard/IME, mouse, wheel, media, popup, download, file picker, and shell overlays.
- Decide BrowserView versus OSR/DirectComposition using measured results.
- Add renderer/browser crash separation and local diagnostics.

Gate: ten representative modern sites render and remain controllable; shell overlays and 4K input stay smooth; websites cannot access the native shell bridge.

### Milestone 2 — instances, profiles, tabs, and browser product core

- Implement instance authentication, profile/request-context isolation, windows, tabs, frames, popup policy, navigation, history, bookmarks, settings, permissions, downloads, and site info.
- Map every existing shell control to real native state.
- Replace “AgentGo Search” with Google and migrate old UI state.
- Bundle fonts/assets and remove shell network dependencies.
- Add internal pages for New Tab, History, Downloads, Settings, Agent Activity, and error/crash states.

Gate: the browser is usable by a human for ordinary browsing without the agent layer and has no fake success behavior.

### Milestone 3 — universal protocol and client isolation

- Implement stdio and named-pipe transports, handshake, ACLs, secrets, versions, schemas, capabilities, events, pagination, backpressure, cancellation, and idempotency.
- Add `agentgo-cli` and protocol conformance tests.
- Connect shell RPC through the same internal command handlers, not a separate privileged implementation.
- Implement leases so clients cannot cross-control profiles/tabs.

Gate: two simultaneous agents control two isolated AgentGo instances with no state or input leakage; an unauthorized client cannot attach.

### Milestone 4 — human-equivalent control

- Implement all P0 pointer, keyboard, selection, focus, clipboard, scrolling, drag/drop, frame, dialog, form, upload, download, media, window, and browser-chrome actions from the inventory.
- Support rich editors, custom comboboxes, shadow DOM, iframes, popups, and native file dialogs.
- Add action proofs and exact typed errors.

Gate: parity suite passes for every P0 primitive using both semantic targets and coordinate fallback where appropriate.

### Milestone 5 — compact observation and grounding

- Build DOM, accessibility, layout, form, content, frame, dialog, and page-metadata extraction.
- Add screenshots/crops/OCR fallback and canvas handling.
- Implement baselines, revisions, diffs, intent filters, stable references, re-resolution, confidence, and ambiguity stops.
- Store large observations as artifacts.

Gate: benchmark pages can be operated from compact state with materially fewer tokens than screenshot/full-DOM loops; stale targets never cause an unverified side effect.

### Milestone 6 — batch workflows and one-call execution

- Implement waits, settle logic, batches, declared postconditions, idempotency, checkpoints, bounded retry/recovery, and structured results.
- Implement deterministic Google search-and-extract, paginate/scroll-and-collect, fill-and-review, upload-and-verify, download-and-verify, and replay workflows.
- Add the external-planner callback session for open-ended tasks.
- Add result schemas, evidence handles, and progress events.

Gate: common workflows complete with one caller request and no hidden model; open-ended tasks request external planning only when needed.

### Milestone 7 — policy, secrets, approvals, and handoff

- Implement centralized scopes and risk classification.
- Add Approval and Full-Access modes exactly as specified.
- Add permanent money-action hard stop.
- Integrate Windows credential storage/DPAPI, autofill, optional explicit visibility, and universal redaction.
- Add CAPTCHA/login/2FA detection, Windows notifications, same-session handoff, resume verification, and timeouts.
- Add page prompt-injection isolation and append-only audit events.
- Add red Stop Agent and reliable Pause/Resume.

Gate: adversarial policy tests prove adapters/pages cannot bypass controls; purchase suite always stops; CAPTCHA suite never attempts a solution and resumes correctly after a human fixture.

### Milestone 8 — artifacts, Windows lane, and secure downloads

- Implement artifact handles, hashes, metadata, previews, redaction, retention, export, and evidence links.
- Implement scoped UI Automation for AgentGo-owned windows and selected native dialogs.
- Implement download progress/pause/resume/cancel/destination/verification/quarantine.
- Integrate Windows Defender scanning where available and refuse automatic execution.
- Implement print/PDF/page capture and crash/trace artifacts.

Gate: upload/download/save/print/native-dialog matrix passes without arbitrary filesystem or desktop control.

### Milestone 9 — skills, replay, and agent adapters

- Ship the bundled skill and update it in lockstep with schemas.
- Implement verified workflow record/replay, parameters, page identity, healing, disabling, and versioning.
- Add CLI, MCP, Rust, TypeScript, and Python adapters generated from the canonical protocol.
- Add domain-adapter format and tests without allowing pages to install adapters.

Gate: Codex, Claude Code, OpenCode, and a generic stdin client all execute the same conformance workflow and receive identical capabilities/policy behavior.

### Milestone 10 — daily-browser P1/P2 coverage

- Complete remaining inventory items: workspaces, session restore, tab groups/pinning/discard, reader/find/translate/source/print, extension/user-script management, media/device flows, DevTools, accessibility settings, low-power/resource budgets, update/rollback, export/import, and advanced debugging.
- Treat Chrome-extension compatibility, Widevine/DRM, proprietary codecs, Google account sync, and Web Store integration as explicit compatibility projects; never claim them from CEF automatically.

Gate: every inventory entry has an honest supported/unsupported status, test, documentation, and skill coverage. Unsupported engine limitations are visible through capability discovery.

### Milestone 11 — performance, security, release, and evaluation

- Threat model, fuzz protocol/parser boundaries, dependency/license audit, secret scan, sandbox validation, crash recovery, update signing, rollback, installer/uninstaller, data deletion, and reproducible build records.
- Run BrowserGym plus AgentGo fixtures for search, forms, editors, iframes, shadow DOM, popups, uploads, native dialogs, downloads, media, stale references, crashes, prompt injection, CAPTCHA handoff, and purchases.
- Measure success, false completion, success/token, model turns, actions, latency, memory, CPU, recovery, and handoff time.

Gate: release candidate meets documented budgets and has no critical security/policy failures, no fake UI action, and no unknown capability status.

## 16. Reference-source extraction map

Do not combine all downloaded projects as runtime dependencies. Port small, tested concepts behind AgentGo-owned interfaces and preserve attribution/license notices.

- `cef`, `cef-project`, `cef-rs`: runtime, lifecycle, Views/OSR, requests, V8/process messages, downloads, DevTools, packaging.
- `ungoogled-chromium`: privacy/build flags reference only.
- `browser-harness`, `browser-use`: session lifecycle, browser ownership, intervention, skills.
- `playwright-mcp`, `agent-browser`, `browser-mcp`: action semantics, selectors, uploads, dialogs, profiles, conformance ideas.
- `drisp-browser-mcp`, `bladebro`, `uiuing-browser-agent`, `malovlab-agent-browser`: compact state, stable IDs, regions, diffs, re-resolution, verification, recovery.
- `browser-agent-native`: compact visual/semantic representation and token budgeting.
- `snact`: snap/action loop, re-snap, persistence, record/replay.
- `bap`, `graph-memory-browser-mcp`: protocol/schema/capability inspiration.
- `windows-harness`: UIA, native dialogs, input/focus truthfulness, proof images.
- `foundry-browser-use`, `i-am-not-a-robot`: intervention state machine and same-session human relay.
- `BrowserGym`: evaluation only.

Before borrowing code, record origin file, commit, license, modification, and destination in `third_party/README.md`. Reimplement concepts when license/provenance is unclear.

## 17. Missing decisions and recommended defaults

These were not settled in brainstorming. The implementation agent should adopt the recommended default, record it as an ADR, and only stop for the user if the choice requires external credentials, money, legal ownership, or publication authority.

1. AgentGo license — recommend Apache-2.0 or dual MIT/Apache-2.0. User must make the final legal choice before public release.
2. CEF release — pin one current stable Windows x64 CEF binary and its Chromium version; update only through tested dependency PRs.
3. Rust versus C++ host — start Rust/cef-rs; permit a minimal C++ shim only for missing binding coverage.
4. Shell compositor — settle through Milestone 1 measurements; BrowserView first, OSR/DirectComposition fallback.
5. Installer/signing — use WiX or MSIX after prototype; public release requires a Windows code-signing certificate supplied by the project owner.
6. Auto-update channel/server — disabled until a signed manifest location and ownership are supplied.
7. Product identifiers — reserve app ID, protocol scheme, publisher, executable names, registry keys, and data paths before installer work.
8. Crash reports/telemetry — local only and off for upload by default.
9. Password vault — Windows Credential Manager/DPAPI first; no custom cross-device vault in v1.
10. Full Chromium source fork — do not download/build until a proven requirement cannot be satisfied by pinned CEF. Track patches separately.
11. Extensions — CEF does not promise full Chrome Web Store behavior. Start with allowlisted unpacked extensions/user scripts; publish a compatibility matrix.
12. DRM/proprietary codecs — out of P0 unless lawful redistributable binaries/licenses are obtained.
13. Google account sync — not part of v1. Google Search is the default; Chrome Sync is a different, restricted product capability.
14. Alternate search providers — optional setting after Google works; never reintroduce “AgentGo Search”.
15. Chrome/Edge fallback — default off, explicit setting, same policy/audit, honest reduction in capabilities.
16. Data retention — recommend 30 days for traces/artifacts and user-configurable history; credentials follow vault lifecycle.
17. Accessibility target — keyboard complete, screen-reader-labelled shell, reduced motion, 200% scale, Windows high contrast.
18. Performance target — 60 FPS foreground UI at 1440p, smooth 4K resize/scroll, idle CPU under 2% on a representative machine, bounded background usage, and no unbounded DOM/log growth.

## 18. Definition of done

AgentGo is not “done” merely because pages render or clicks work. Release-level completion requires:

- the approved UI running as the real browser shell;
- Google as primary/default search and no AgentGo Search;
- every UI control backed by real state or honestly disabled;
- universal local protocol with equal capabilities for all agents;
- isolated authenticated instances and profiles;
- complete P0 human-equivalent browser and scoped native control;
- semantic/visual observation, stable refs, deltas, batching, verification, recovery, and artifacts;
- deterministic one-call workflows plus external planner sessions;
- correctly enforced modes, purchase gate, CAPTCHA handoff, credential rules, and Stop/Pause;
- bundled skill, conformance tests, examples, and generated SDKs;
- comprehensive security, license, performance, crash, and benchmark gates;
- capability inventory with no undocumented or falsely implemented entries.

## 19. Instructions for the implementation agent

1. Read this plan, both existing output documents, the approved UI, and repository-local instruction files before editing.
2. Work milestone by milestone. Do not attempt the entire project in one uncontrolled patch.
3. At the start of each milestone, create or update a checklist tied to capability IDs and tests.
4. Preserve the research archive as read-only reference; build AgentGo in a fresh repository.
5. Prefer the smallest safe implementation that satisfies the public contract, but never use mocks as final behavior.
6. Run relevant unit, integration, security, visual, and performance checks before moving to the next gate.
7. Keep the skill, schemas, implementation, documentation, and tests synchronized in every capability change.
8. Do not silently weaken a user requirement because CEF makes it difficult. Expose an honest capability limitation and continue with the next safe path.
9. Do not perform purchases, publish releases, buy/sign certificates, choose legal licensing for the owner, or enable cloud services without explicit user authority.
10. Report concrete evidence: files changed, commands run, tests passed, remaining failures, capability status changes, and the next gate.
