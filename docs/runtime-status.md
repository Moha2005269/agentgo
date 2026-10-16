# AgentGo Runtime Handoff

Updated: 2026-09-09

This is the operational handoff for the CEF runtime foundation. Keep it current
when a milestone changes behavior. It is intentionally concrete so a new
agent can resume without relying on chat history.

## What is working now

- The Windows CEF runtime launches through agentgo.exe run and through a
  bare double-click when the binary was built with the normal Runtime
  feature.
- Normal desktop launches use CEF's built-in message loop and hardware
  compositor. Stdio and smoke harnesses use the bounded external-pump path.
  Both preserve the trusted `agentgo://shell` BrowserView separately from the
  untrusted content BrowserViews.
- The shell is served from bundled ui/chrome.html resources with an
  explicit content type, CSP, and no-store headers. The content view opens
  the requested URL after the shell is ready; a normal bare desktop launch
  opens the approved landing page while an agent stdio launch remains
  Google-first.
- The shell bridge is restricted to the exact trusted shell origin and main
  frame. Page content does not receive the native bridge.
- A stdio protocol transport is available for local agent clients. It keeps
  protocol JSON on stdout and runtime diagnostics on stderr.
- The first real request handlers are implemented and exercised:
  api.health, api.version, api.capabilities, browser.health,
  browser.version, browser.capabilities, tabs.navigate, tab.navigate,
  tabs.list, tab.list, tabs.status, tab.status, tab.reload, tab.stop,
  tab.back, tab.forward, window.minimize, window.maximize, window.resize,
  browser.shutdown, window.close, pointer.move, pointer.click,
  pointer.scroll, keyboard.type, keyboard.press, downloads.list,
  downloads.clear, download.start, download.progress, download.pause,
  download.resume, and download.cancel.
- Unknown method names return method.notFound. Methods known to the
  registry but not implemented yet return capability.unsupported.
  Invalid URLs and malformed envelopes return typed protocol errors.
- The runtime exposes stable native content tab ids. `tabs.create` creates a
  BrowserView in the shared isolated content context, `tabs.activate` changes
  visibility/focus and relayout, and `tabs.close` detaches any selected tab
  while at least one other tab remains. No first-tab exception exists.
  `tabs.*` and `tab.*` names are accepted as aliases.
- Smoke mode has a bounded 30-second timeout and accepts a local file
  fixture, so a denied network request cannot leave the test process hanging.
- The shell asset path uses a deterministic in-memory CEF resource handler:
  it sets MIME type and charset through the CEF response API, writes the
  Content-Type/CSP headers directly, reports the exact body length, and uses
  a versioned shell query string to avoid stale custom-scheme cache entries.
- The shell controls are served as same-origin `ui/chrome.js`, so the CSP can
  keep `script-src 'self'` without silently blocking the address bar and
  reload handlers.
- Every normal or agent launch receives a unique temporary Chromium root under
  `%TEMP%\agentgo-runs\<pid>-<nonce>`. This prevents ProcessSingleton
  collisions between parallel isolated sessions and stale child processes.
- Normal desktop launch opens the approved `ui/index.html` landing surface
  below the trusted chrome. Stdio launches still open Google by default. The
  landing document has a scoped CSP and cannot invoke `__bridge`; the bridge
  requires the main `chrome.html` frame.
- Settings are local, profile-scoped, versioned atomic records. Durable
  history is stored in `data/state.db` with SQLite WAL enabled; the protocol
  exposes only bounded URL/title metadata. `AGENTGO_DATA_ROOT` is available
  as a test/portable override, and the settings writer rejects
  credential-shaped keys.
- Each process publishes `instances\\<instance_id>\\endpoint.json` using an
  atomic record. It contains only lifecycle metadata (pid, protocol,
  transport, profile, and stopped status); instance secrets, cookies, and
  page content are never written. Authenticated named-pipe attachment remains
  a Milestone 3 transport.
- The isolated content request context registers the bundled `agentgo://`
  asset factory explicitly. Desktop smoke proves the landing document reaches
  HTTP 200 in that context; the landing document still cannot invoke the
  native bridge.
- The approved landing surface bundles its Latin IBM Plex Mono, Instrument
  Serif, and Manrope faces under `ui/fonts`, with local OFL notices; no font
  CDN request is required for first paint.
- Native window minimize and maximize/restore actions are implemented and
  return a post-action state snapshot.
- Native window resize is implemented with bounded dimensions and returns the
  accepted size plus the post-action window state. The existing window
  delegate relayouts the shell and content BrowserViews after the native
  resize.
- Basic native input is implemented for the active content view. Pointer
  coordinates are viewport-local and bounded; keyboard text is sent as
  UTF-16 CEF CHAR events and named key presses as KEYDOWN/KEYUP pairs.
- CEF download lifecycle tracking is implemented behind the explicit
  `settings.downloadsEnabled` switch (disabled by default). Downloads are
  written only to the scoped per-run/profile directory; agents can explicitly
  start a URL download, list records, read bounded progress metadata, pause,
  resume, cancel, and clear completed records. Files are never executed
  automatically.
- Renderer unresponsive/termination callbacks are now separated from the
  browser lifecycle. A bounded redacted report is retained in memory and
  written as a local JSON artifact under `data\crashes`; `browser.crash_report`
  returns metadata and the artifact path, never page content or request bodies.
- Content media and permission prompts default-deny. Native file chooser
  requests are recorded as safe metadata and then delegated to CEF's native
  Windows picker; arbitrary desktop windows are not controlled by this slice.

## Reproducible commands

Run these from the repository root:

    . .\scripts\env.ps1
    Ensure-CargoOnPath
    cargo test --workspace --locked --offline
    .\scripts\build.ps1 -Runtime
    .\target\debug\agentgo.exe run --smoke --url https://example.com/
    .\target\debug\agentgo.exe run --smoke --url file:///C:/path/to/fixture.html
    node .\tests\integration\protocol_smoke.js

For an agent process, use:

    .\target\debug\agentgo.exe run --stdio --url https://example.com/

Send newline-delimited protocol requests on stdin and read only
newline-delimited JSON responses from stdout. Human-readable runtime events
are on stderr.

### Launch/build guard

The direct desktop executable is the runtime-feature build. Always rebuild it
with `.\scripts\build.ps1 -Runtime` before launching
`target\debug\agentgo.exe`. A plain `cargo build` can overwrite the same
artifact with the CLI-only binary; that binary exits with code 2 on a
double-click and never creates a CEF window. The runtime build is the one
that opens the trusted chrome, the approved landing page, and the isolated
content view.

## Sandbox status

`.\scripts\build.ps1 -Runtime -Sandbox` now builds the CEF bootstrap shape:
`bootstrap.exe` is packaged as `target\agentgo-sandbox\debug\agentgo.exe`,
the Rust `agentgo.dll` exports `RunWinMain`, and the bootstrap-provided
`sandbox_info` is forwarded to CEF child-process dispatch and initialization.
The package smoke on 2026-09-08 stayed alive, registered the shell scheme,
loaded `chrome.html`, loaded `chrome.js`, loaded `index.html`, and reached
`content-loaded ... status=200 agentgo://shell/index.html` before clean stop.

The direct Rust `target\debug\agentgo.exe` built with the sandbox feature
still fails closed by design; only the bootstrap package is a valid sandbox
entry path. The checked-in CEF distribution does not need a separately linked
`cef_sandbox.lib` for this official bootstrap route.

## Evidence and known limitations

- The 2026-09-09 composition smoke logged the trusted shell at
  `1360x104+0,0` and the content BrowserView at screen bounds
  `1360x796+0,104`. This verifies that the page starts below the chrome in
  native screen coordinates rather than being painted beneath it and clipped.
- The 2026-09-09 rebuild moved History, Downloads, and Settings into the
  separate bounded `ui/tools.html` BrowserView. Lockstep tests assert that
  `chrome.js` invokes the private `ui.open_tool` bridge while the landing page
  has no tool dispatcher, preventing utility clicks from replacing content.
- cargo test --workspace --locked --offline passed after the runtime
  changes.
- `node .\tests\integration\protocol_smoke.js` passed again after the final
  M0-M2 shortcut/overlay pass. It created a second native tab, restacked the
  browser-tool surface, switched tabs, closed the second tab, and verified
  that the final remaining tab is protected.
- node .\tests\integration\protocol_smoke.js passed: strict stdout JSON
  parsing, health/version/capabilities, unknown and planned-method errors,
  invalid URL rejection, navigation, status, profile lifecycle, settings,
  history, Google search resolution, site metadata, tab actions, native
  pointer/keyboard dispatch, download start/progress/gating/lifecycle, window
  controls and resize, crash-report metadata, permission policy, download
  controls, and clean shutdown.
- Desktop launch smoke passed: the normal runtime stayed alive for 8 seconds
  and logged `AG-SCHEME serve index.html` followed by
  `AG-RUNTIME content-loaded status=200 agentgo://shell/index.html`.
- `node .\tests\integration\site_render_smoke.js` passed on ten public sites:
  example.com, Google, GitHub, Mozilla, W3C, Wikipedia, Rust, Chromium,
  browser-use.com, and Stack Overflow. Each navigation settled, remained
  addressable by the active tab, accepted `pointer.move`, and the process
  exited cleanly. Stack Overflow served a normal 403 page; it still settled
  and remained controllable.
- `node .\tests\integration\protocol_smoke.js` passed after adding the
  3840×2160 resize and pointer check, then restoring 1200×800.
- The public smoke fixture used for repeatable validation is
  https://example.com/ plus the local fixture selected by the harness.
- The full approved `ui/index.html` landing surface is now live for normal
  desktop launch. It remains a local product surface; its prototype controls
  that do not appear in the implemented protocol list must not be described
  as native browser features yet.
- A previous visual failure showed the browser source text directly. The
  cause was the custom scheme response not setting a reliable explicit
  Content-Type; the response now sets the MIME header and CSP, and the
  rebuilt normal runtime logs chrome.html as text/html before shell load. The
  latest rebuild also removes the CSP inline-script violation and the host
  remains responsive past the previous one-second failure window. The current
  composition smoke now covers the normal and bootstrap package paths.
- CEF may still print nonfatal local warnings about OS encryption,
  Google Update registry access, geolocation parameters, and observer
  registration. They are not proof that the runtime failed.
- Windows desktop launches use CEF's built-in message loop and real hardware
  acceleration. The executable embeds the modern Windows compatibility and
  PerMonitorV2 manifest required by Chromium GPU detection. Set
  `AGENTGO_RENDER_MODE=software` or `AGENTGO_RENDER_MODE=swiftshader` only as a
  temporary diagnostics fallback.
- The trusted chrome tab strip is wired to the native `tabs.create` and
  `tabs.activate` bridge paths; tab creation is queued onto CEF's UI thread
  before a new BrowserView is attached.
- New chrome tabs open the bundled AgentGo landing page, and each tab exposes
  a close control while the final remaining tab is protected. Back, forward,
  reload, minimize, maximize/restore, and close-window controls are wired to
  UI-thread tasks rather than touching CEF window/browser objects from the
  scheme handler thread.
- Landing bookmarks and shortcut tiles use ordinary HTTPS links to their real
  destinations (including arXiv, GitHub, Google Mail, Figma, Maps, YouTube,
  Hacker News, and Google Docs). They navigate inside the isolated content
  tab and receive no trusted-shell bridge access.
- Native tab snapshots are pushed into the trusted chrome after navigation,
  activation, creation, closing, and load completion. This keeps the address
  bar and tab labels synchronized with redirects and removes the need for
  high-frequency JavaScript polling.
- Content-view visibility is keyed by the stable tab-id order, not CEF wrapper
  identity comparisons. This keeps the approved landing surface and remote
  pages visible after startup, navigation, resize, and tab activation.
- The top-level CEF Window owns a vertical BoxLayout. `ui/chrome.html` is its
  fixed 104-DIP first child and the active content BrowserView is the flex child
  below it. New tabs are added directly to that layout, hidden tabs do not
  consume space, and the active site retains its complete top-of-page UI.
- History, Downloads, and Settings render in a separate trusted
  `ui/tools.html` BrowserView. Its native overlay is bounded to only the
  popover or settings-drawer rectangle, so the page remains visibly present
  around it instead of becoming a black transparent-compositor surface.
  Transform/opacity transitions run inside the bounded view; close waits for
  that transition before `ui.close_tool` hides the native overlay.
- Creating a content tab recreates the bounded utility overlay after the new
  child View. This preserves CEF's high-z-order guarantee, so History,
  Downloads, and Settings remain visible over landing and remote pages alike.
- The bundled landing URL retains an `embedded=1` marker that hides only its
  duplicate prototype title/address rows.
- The stdio reader starts only after the global runtime state is installed;
  this avoids an early-agent-request race that previously could panic the
  reader thread before the first navigation response.

## Work intentionally deferred after the M1/M2 foundation gate

1. Rebuild and rerun the normal runtime smoke after any sandbox compile.
2. Extend the current basic input/composition slice to full DPI,
   keyboard/IME, touch/pen, wheel, popup, DevTools, and overlay coverage.
3. Add browser-level recovery/restart, bounded resource budgets, structured
   diagnostics, and evidence artifacts.
4. Promote only genuinely complete handlers in
   schemas/capability-manifest.json; keep the generator, registry, tests,
   and skills/ documentation in lockstep.
5. Implement authenticated Windows named-pipe transport, session leases,
   policy/approval gates, audit logging, multi-tab profiles, and the remaining
   workflow/observe/locate/action surfaces only when the project explicitly
   begins the later milestones.
6. Update this file, docs/milestones.md, docs/architecture.md, and
   tests/integration/README.md in the same change as each milestone gate.

## Safety boundary for testing

Use public fixtures and local pages for automated testing. Do not submit
forms, purchase anything, bypass CAPTCHA, or operate a personal account
without an explicit interactive user handoff.
