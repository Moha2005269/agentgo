# AgentGo Compatibility

Status: Milestones 1-2 foundation. This matrix records what the pinned engine provides
and what AgentGo honestly will not claim.

## Engine

| Capability | Status |
|---|---|
| Engine | CEF `152.0.5+gb129680+chromium-152.0.7977.54` (Chromium 152.0.7977.54), Windows x64, pinned and hash-verified |
| Bindings | cef-rs `152.0.0+152.0.5` @ git `83640144407b5742356b3ea6678a8b2de921e51d` |
| Sandbox | Official CEF `bootstrap.exe` + Rust `RunWinMain` package verified; direct sandbox-feature EXE still fails closed by design |
| Process model | Chromium multi-process; renderer unresponsive/termination separation and redacted local reports implemented |

## Explicitly NOT claimed from CEF

| Area | Honest position |
|---|---|
| Chrome Web Store extensions | CEF does not promise full Web Store behavior. Start with allowlisted unpacked extensions/user scripts; publish a compatibility matrix (M10). |
| DRM/Widevine, proprietary codecs | Out of P0 unless lawful redistributable binaries/licenses are obtained. |
| Google account sync | Not in v1. Google Search is the default search provider; Chrome Sync is a different, restricted capability. |
| Chrome/Edge fallback | Optional, default-off user setting; reported as an honest handoff/fallback with the same policy/audit and a documented capability reduction. |

## Search

- Google is the default and primary search provider (ADR-0006). No
  AgentGo Search, no search proxy, no hosted backend.
- Alternate providers: optional setting only after Google works.

## Fonts/assets

- The approved UI bundles its Latin fonts locally under `ui/fonts` with OFL
  notices; no Google Fonts network request is required for first paint.
