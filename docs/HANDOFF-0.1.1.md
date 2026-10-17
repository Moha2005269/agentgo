# Handoff: 0.1.1 is built and tested, not pushed or released (2026-10-17)

Work happens in `C:\Users\m7md0\Downloads\AgentGo-backup-2026-10-15\agentgo-remote`
(a clone of github.com/Moha2005269/agentgo, branch `main`, private repo,
keep it private). Nothing below is committed or pushed yet.

## Test status of the current working tree (debug build)

- build: OK
- clippy (workspace; and `-p agentgo-app --features runtime`; and `installer/`): OK, no warnings
- unit tests (`cargo test --workspace`): all pass
- integration (`node scripts\dev\run-integration.js`): **34/34 passed**
- live (`--live security_live_e2e`): passed earlier today (badssl 5/5, WebRTC srflx only,
  adblock-tester 100/100, EFF strong protection, dangerous sites blocked, GPC, etc.)
- No open issues from the last rebuild.

## Done this session (see CHANGELOG.md 0.1.1 and docs/issues-log.md D78-D90)

CI fix (manifest hash/line endings, lockstep test); fingerprint protection hardened and
mode switching fixed; WebRTC public-only; dangerous-site blocking (35k hosts + look-alike
detection, offline); 3P cookies blocked, tracking params stripped, GPC; import fixes
(Chrome account passwords, guided export, `agentgo import`); MCP fixes (Claude user scope,
Droid path); DevTools fixes (close button, re-inspect, toolbar no longer disappears,
anchor devtools); extensions (store button routes into AgentGo, chrome.* shim, legacy
1024-bit CRX); memory (back/forward cache off: 2.2 GB -> 1.4 GB); page-visible markers
removed; windows open centred; **auto-update** (checks GitHub latest release, signed
manifest, toolbar button, Settings switch "Update automatically", film-style update via
`AgentGo-Setup.exe --update`, user data folder never touched); the owner's film installer
(`installer/ui/index.html`, from C:\Users\m7md0\agentgo-installer-film) with silent mode;
README blue download button; benchmark results in docs/benchmarks/browsers-benchmark.md.

Version is 0.1.1 (Cargo.toml, installer/Cargo.toml, manifest regenerated).

## Still to do, in this order

1. **Release build** (old `dist\AgentGo-0.1.1-win64*` is stale, delete it first):
   ```
   powershell -File scripts\build.ps1 -Runtime -Release
   powershell -File scripts\build.ps1 -Runtime -Sandbox -Release
   powershell -File scripts\package.ps1 -Release -UpdateBase "https://github.com/Moha2005269/agentgo/releases/download/v0.1.1/"
   powershell -File scripts\build-installer.ps1
   ```
   package.ps1 signs `dist\update-manifest.json` with the key in
   `%USERPROFILE%\.agentgo-signing\update-ed25519.pem` (present on this PC; must say "signed").
   Outputs: `AgentGo-0.1.1-win64.zip` (+ .sha256), `update-manifest.json`,
   `AgentGo-Setup.exe`, `AgentGo-Setup-full.exe` (+ .sha256).
2. **Test a real install** with `dist\AgentGo-Setup-full.exe --silent --dir <temp folder>`,
   check `%TEMP%\agentgo-setup.log`, start the installed browser once, then
   `--uninstall --silent`. (Do not install over the owner's own copy.)
3. **Commit and push** to `main` (no force push). Do not commit `dist\`, `target\`,
   `installer\target\` (gitignored). Check `git status` for stray temp files first.
   Commit message ends with the attribution line required by the session.
4. **Watch CI** (`.github/workflows/ci.yml`; it now also builds the installer and parses UI JS).
5. **Publish release `v0.1.1`** with assets: the zip and its .sha256,
   `update-manifest.json`, `AgentGo-Setup.exe`, `AgentGo-Setup-full.exe` and their .sha256.
   Release notes draft: scratchpad `release-notes.md` (or summarise CHANGELOG 0.1.1).
   The asset names must match exactly: the README buttons link
   `releases/latest/download/AgentGo-Setup.exe` / `AgentGo-Setup-full.exe`, and
   auto-update reads `releases/latest/download/update-manifest.json`.
6. Remind the owner to **revoke the GitHub token** pasted in chat after pushing.

## Known limits (documented, not bugs)

- CreepJS "20% stealth" with fingerprint protection on: JS Proxies are detectable;
  real fix needs engine-level (C++) noise = custom Chromium build.
- Extensions: page scripts + chrome.runtime/i18n/storage only; no background pages/pop-ups.
- Chromium 152 (current 156): a CEF update is a separate task.
- BrowserAudit 34 warnings = same as stock Chromium (outdated tests).
- Benchmark toolkit lives in `C:\Users\m7md0\.agentgo-bench` (outside the repo); adapter
  copies are in `scripts/dev/browsers-benchmark/`.
- Auto-update only works for others once the repo is public (owner's decision, later).
