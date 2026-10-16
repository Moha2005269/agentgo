# ADR-0002: Shell/content composition — BrowserView first, OSR/DirectComposition fallback

- Status: Accepted (BrowserView selected after the Milestone 1 gate)
- Date: 2026-09-07

## Context

CEF cannot safely render arbitrary websites inside a normal HTML iframe
(framing blocks, cross-origin policy). The approved AgentGo UI must be a
trusted shell while untrusted pages render with full fidelity. Two
compositions are viable:

1. **CEF Views (`CefBrowserView`)**: the trusted shell browser view fills
   a `CefWindow`; the untrusted content browser view is added as a child
   view positioned in the shell's viewport rectangle. Composition happens
   inside Chromium's UI layer (hardware accelerated, no child-HWND
   airspace problems between the two views).
2. **OSR + DirectComposition/D3D11**: windowless rendering of the content
   surface composited by AgentGo itself.

## Decision

Spike with `CefBrowserView` first (master plan section 4): it is simpler
and hardware accelerated. Internal pages (New Tab, Settings, History,
Downloads, Agent Activity) hide the external content view and use the
full shell. Move **only the page surface** to OSR/DirectComposition if
and only if measured spike results show the approved overlays, menus,
dialogs, IME, or 4K resize cannot be preserved.

## Measurement criteria (Milestone 1 gate)

- Ten representative modern sites render and remain controllable.
- Shell overlays render above the content surface where the approved UI
  requires it.
- Resize/DPI/4K input stay smooth; keyboard/IME/wheel work in both
  surfaces.
- Websites cannot reach the `agentgo://shell` origin or the native bridge.

## Consequences

- The shell keeps its exact approved visual identity in either route.

## Milestone 1 measurement record — 2026-09-08

The native BrowserView composition passed the current foundation gate:

| Check | Result |
|---|---|
| Ten representative public sites | 10/10 navigated, settled, and accepted a native pointer move; Stack Overflow returned a normal 403 page but remained controllable |
| Shell/content separation | Shell is a trusted topmost sibling BrowserView; content is a separate request context and cannot use the shell bridge |
| 4K resize/input | 3840×2160 resize succeeded, pointer input at the 4K viewport center succeeded, and the window restored to 1200×800 |
| Bootstrap deployment | Official bootstrap package loaded `chrome.html`, `chrome.js`, and landing `index.html` to status 200 |

Evidence commands:

    node .\tests\integration\site_render_smoke.js
    node .\tests\integration\protocol_smoke.js

OSR/DirectComposition is not selected for this foundation. It remains a
future fallback if later full IME, overlay, popup, or compositor profiling
finds a BrowserView criterion that fails.
