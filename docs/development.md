# AgentGo Development Guide

## First run on a clean Windows 11 x64 machine

```powershell
git clone <repo> agentgo
cd agentgo

# 1. Install/verify the pinned toolchain and fetch dependencies
powershell -ExecutionPolicy Bypass -File scripts\bootstrap.ps1 -Stage toolchain
powershell -ExecutionPolicy Bypass -File scripts\bootstrap.ps1 -Stage fetch

# 2. Verify the environment
powershell -ExecutionPolicy Bypass -File scripts\doctor.ps1

# 3. Build (debug) and run the tests
powershell -ExecutionPolicy Bypass -File scripts\build.ps1
powershell -ExecutionPolicy Bypass -File scripts\test.ps1

# Release build
powershell -ExecutionPolicy Bypass -File scripts\build.ps1 -Release
target\release\agentgo.exe version
```

`scripts\bootstrap.ps1 -Stage cef` (included in `-Stage all`) downloads
the pinned CEF binary distribution (~343 MB), verifies size + SHA-1 (+
SHA-256 once recorded), and lays it out for the Milestone 1+ builds.

## Git long paths

The repository sets `core.longpaths=true` on clone via the setup step
below; CEF/Chromium paths can exceed 260 characters. On a fresh clone:

```powershell
git config --global core.longpaths true
```

(and enable Windows long-path support in group policy/registry if your
organization restricts it) before building anything that touches the CEF
distribution tree.

## Everyday commands

| Task | Command |
|---|---|
| Format check / fix | `scripts\fmt.ps1` / `scripts\fmt.ps1 -Fix` |
| Lint (clippy, warnings denied) | `scripts\lint.ps1` |
| Tests (unit + integration + conformance) | `scripts\test.ps1` |
| Doctor | `scripts\doctor.ps1` |
| Regenerate capability manifest | `scripts\regen-manifest.ps1` |
| Version report | `cargo run -p agentgo-app -- version` |

## Synchronization rules (enforced by tests)

- `docs/capability-inventory.md` is the command-level source of truth;
  `schemas/capability-manifest.json` is generated from it and committed.
  `agentgo manifest validate` (and the lockstep integration test) fail
  when out of sync.
- A capability may only be marked `implemented` in the manifest in the
  same commit that lands its handler and test id.
- `scripts/pins.json`, `crates/agentgo-cef/src/pins.rs`, and the
  workspace `Cargo.toml` cef-rs revision must agree
  (`pins_json_agrees_with_code_constants`).
- The bundled skill (`skills/agentgo/SKILL.md`) must declare the same
  protocol/skill versions and reference the manifest
  (`skill_declares_the_same_versions_and_search_policy`).
- The approved UI file is tracked at `ui/index.html`; its prototype-only
  behaviors are pinned as Milestone 2 migration obligations
  (`approved_ui_is_preserved_and_uses_google_search_only`).

## Branch/commit conventions

- One milestone per merge; every capability-status change cites its test
  id in the commit body.
- Never replace behavior with mocks, timers, hard-coded data, or success
  toasts; unsupported controls are visibly disabled with a reason.
- Research references stay read-only outside this repository
  (`work/agent-browser-sources/`); extraction rules are in
  `third_party/README.md`.
