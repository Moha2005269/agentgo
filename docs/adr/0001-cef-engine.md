# ADR-0001: CEF/Chromium is the active engine

- Status: Accepted
- Date: 2026-09-07
- Deciders: planning handoff (master plan section 2), implementation agent

## Context

AgentGo needs a Windows-first rendering engine with modern-site
compatibility, a stable embedding API, process sandboxing, and room for
deep agent instrumentation. The research archive (read-only, at
`work/agent-browser-sources/`) evaluated Servo, Verso, Zervo, Ladybird,
WebKit/Windows, Gecko, and WebView2 wrappers. None match Chromium's
Windows production maturity and modern-site compatibility today.

## Decision

Use CEF (Chromium Embedded Framework) binary distributions, pinned to an
exact version, with a Rust-native AgentGo control core built on the
`cef-rs` bindings (pinned to an exact git revision). Servo and the other
engines remain research-only. A full Chromium source fork is explicitly
deferred until a proven requirement cannot be satisfied by pinned CEF.

## Consequences

- Modern-site compatibility follows Chromium 152 (pinned: CEF
  `152.0.5+gb129680+chromium-152.0.7977.54`).
- The engine is a **binary dependency**, verified by size + SHA-1 +
  SHA-256 in `scripts/pins.json` and `crates/agentgo-cef/src/pins.rs`.
- DRM/Widevine, Chrome Web Store behavior, and Google account sync are
  **not** inherited automatically; they are separate compatibility
  projects with honest status in the capability manifest.
- If a Blink/content-level patch becomes a proven requirement, the CEF
  branch can be expanded into a pinned source build via depot_tools; that
  is a separate infrastructure decision.
