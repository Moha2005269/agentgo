<p align="center">
  <img src="assets/hero.svg" alt="AgentGo, the browser that fits you and your agent" width="100%">
</p>

<p align="center">
  <a href="https://github.com/Moha2005269/agentgo/releases/latest"><b>Download for Windows 11</b></a>
  &nbsp;·&nbsp; <a href="#watch-an-agent-work">Watch it work</a>
  &nbsp;·&nbsp; <a href="#connect-your-agent">Connect your agent</a>
  &nbsp;·&nbsp; <a href="#security-and-privacy">Security</a>
  &nbsp;·&nbsp; <a href="#build-from-source">Build</a>
</p>

<p align="center">
  <img alt="Windows 11 x64" src="https://img.shields.io/badge/Windows%2011-x64-ff6a2b?style=flat-square">
  <img alt="Chromium 152" src="https://img.shields.io/badge/Chromium-152-ff6a2b?style=flat-square">
  <img alt="Made with Rust" src="https://img.shields.io/badge/made%20with-Rust-ff6a2b?style=flat-square">
  <img alt="License BSL 1.1" src="https://img.shields.io/badge/license-BSL%201.1-ff6a2b?style=flat-square">
  <img alt="No telemetry" src="https://img.shields.io/badge/telemetry-none-ff6a2b?style=flat-square">
</p>

---

## A browser for you, and for the agent next to you

AgentGo is a real Chromium browser for Windows with no ads, no tracking and no accounts to create. It is also built so an AI agent (Claude Code, Codex, any MCP host or your own program) can use it the way a person would, while you watch and stay in charge.

AgentGo makes no model calls and needs no API key. Your agent does the thinking. AgentGo does the browsing, and answers in a few lines of text instead of a wall of HTML.

<table>
<tr>
<td width="50%" valign="top">

### Strong ad and tracker blocking
Brave's open-source blocking engine with **18 filter lists**: EasyList, EasyPrivacy, uBlock Origin, AdGuard, Peter Lowe, URLhaus, HaGeZi and more. It blocks requests, hides leftover page elements, runs scriptlets, strips YouTube ads at the network level and clears cookie banners. It is on by default and has a live counter. Public comparison benchmarks are coming; we will publish them when they are done.

</td>
<td width="50%" valign="top">

### A browser that fits you and your agent
The agent works with its own pointer and keyboard, drawn on the page and never your mouse, in a window that stays behind yours. You see every click and keystroke, and you can pause or stop it in one click. Everything else is a normal browser: tabs, bookmarks, history, downloads, extensions from the Chrome Web Store.

</td>
</tr>
<tr>
<td width="50%" valign="top">

### Security and fingerprint protection
Sandboxed renderers at untrusted integrity, per-session fingerprint noise, site permissions that stay blocked until you allow them, signed updates, DevTools gated, a prompt-injection scanner on page text, and an audit log that never records typed text or cookies. We aim to be one of the most secure and private browsers you can run. Independent benchmarks are coming.

</td>
<td width="50%" valign="top">

### Accounts for your agent, safely
Create accounts just for your agent, or let it use yours. Saved sign-ins are encrypted for your Windows user and **typed for the agent without it ever seeing the password**. Block any account in the Account Center, and the agent cannot touch it. Payments always ask you, in every mode. CAPTCHAs and one-time codes come to you in the same tab.

</td>
</tr>
<tr>
<td colspan="2" valign="top">

### Laya, a model that knows the browser
Laya is a small custom model we are pre-training on how this browser works, so agents spend fewer tokens and browsing feels effortless. **Status: in training.** The interface is already in the browser (`decision.record` and `decision.apply`, plus step recording and metrics for training data), and the model itself is not bundled yet.

</td>
</tr>
</table>

## Watch an agent work

<p align="center">
  <img src="assets/demo.svg" alt="Animated demo: an agent reads a checkout page as a short outline, fills the sign-in form, waits for you to approve a payment, then finishes the order" width="100%">
</p>

The agent never reads raw HTML. It gets a short outline with numbered references, and every action answers with only what changed. That is why the demo page costs **186 tokens instead of about 42,000**.

```text
observe.page →
# Sign in to check out
[1] textbox(email) "Email" required
[2] textbox(password) "Password" required
[3] checkbox "Keep me signed in"
[4] button "Continue"

act.fill {ref: 1, text: "…"} →
~ [1] textbox(email) value="…"
```

## Everyday browsing, done well

- **Command palette** (Ctrl+K): search settings, tabs, history, bookmarks and actions.
- **First-run welcome and import** of history, bookmarks and open tabs from Chrome, Edge, Brave and Firefox.
- **Dark mode for websites** with brightness and contrast sliders and a per-site exclusion list.
- **Translation** of a selection or a whole page into the language you pick (Settings → Languages).
- **Site permissions that ask**: camera, microphone and location are blocked until you choose *Allow this time* or *Always allow*, and you can block a site again any time.
- Favorites bar, find in page, zoom, print and print to PDF, session restore, tab groups, pinned tabs, several windows.
- **Report a problem** saves a diagnostics zip on your machine. Nothing is sent anywhere.

## Install

1. Download `AgentGo-<version>-win64.zip` from the [latest release](https://github.com/Moha2005269/agentgo/releases/latest) and check it against the published SHA-256.
2. Unzip it anywhere and run `agentgo.exe`. It works from the folder and installs nothing.
3. Optional: run `Install-AgentGo.ps1` for a Start-menu entry, PATH and Claude Code registration (current user only, no administrator rights, `-Uninstall` removes it).

```powershell
powershell -ExecutionPolicy Bypass -File Install-AgentGo.ps1
```

Your profile (history, bookmarks, cookies, saved sign-ins) lives under `%LOCALAPPDATA%\AgentGo`. The executables are not Authenticode-signed yet, so Windows SmartScreen may ask once.

## Connect your agent

```powershell
claude mcp add agentgo -- C:\path\to\agentgo.exe mcp     # any MCP host
agentgo.exe call observe.page                              # any shell
agentgo.exe run --stdio                                    # programs: JSON lines
```

Agents should read `skills/agentgo/SKILL.md` first (over MCP: `browser_guide`). Every command is listed in `skills/agentgo/references/commands.md`. `agentgo setup droid|claude|codex|json` registers AgentGo with an agent host for you.

## Security and privacy

- **No telemetry, no ads, no accounts.** AgentGo does not phone home. What it connects to is documented in [`docs/NETWORK_CALLS.md`](docs/NETWORK_CALLS.md).
- **Sandboxed**: web pages render in untrusted-integrity processes (CEF `bootstrap.exe` + `agentgo.dll`).
- **Fingerprint protection**: per-session noise on the signals trackers use to recognise you.
- **You stay the admin**: payments always ask, CAPTCHAs are handed to you (never solved), every action is logged locally, and typed text and cookie values never enter the log.
- **Signed updates**: updates are opt-in and only accepted when signed with the release key (Ed25519).
- **Prompt-injection scanner**: page text given to an agent is checked for instructions aimed at the agent.
- **How each claim is tested**: [`docs/SECURITY_TESTING.md`](docs/SECURITY_TESTING.md). Threat model: [`docs/threat-model.md`](docs/threat-model.md). Report a vulnerability: [`SECURITY.md`](SECURITY.md).

## Honest limits

- Windows 11 x64 only.
- The ad blocker and security claims above come from our own testing. Independent benchmarks against other browsers are not published yet.
- Import does not bring over Firefox passwords or Chrome's newest (v20) encrypted passwords.
- Translation uses Google's translation service. Notifications are refused for every site. There is no VPN in this release.
- Cross-origin iframes are not observed or acted in.
- Laya is not bundled yet.
- Real Google, GitHub and Yandex API sign-in has not been tried live; the OAuth flow is tested against a mock provider.

## Build from source

```powershell
powershell -ExecutionPolicy Bypass -File scripts\bootstrap.ps1 -Stage toolchain
powershell -ExecutionPolicy Bypass -File scripts\build.ps1 -Runtime -Release              # tools
powershell -ExecutionPolicy Bypass -File scripts\build.ps1 -Runtime -Sandbox -Release     # sandboxed browser
powershell -ExecutionPolicy Bypass -File scripts\package.ps1 -Release
```

Tests: `cargo test --workspace --offline` and `node scripts\dev\run-integration.js` (end-to-end files on a private desktop). Engine: CEF 152 (Chromium 152.0.7977.54), pinned and hash-verified; Rust 1.98.1 MSVC; cef-rs pinned by commit.

Layout: `crates/` (Rust workspace), `ui/` (browser chrome and panels), `skills/agentgo/` (the agent's guide), `tests/integration/`, `docs/`.

## Why we built it, and thank you

We do not care about ads, and we do not care about your data. AgentGo is not built for profit and nothing here is designed to make money from its users. It is built for humanity, on the shoulders of decades of open-source work.

Thank you to:

- **Brave** for [adblock-rust](https://github.com/brave/adblock-rust) and [adblock-resources](https://github.com/brave/adblock-resources), the engine behind our blocker.
- **The Chromium project and the CEF team** (Marshall Greenblatt and contributors), and **Tauri** for [cef-rs](https://github.com/tauri-apps/cef-rs).
- **Raymond Hill and the uBlock Origin community**, **EasyList and EasyPrivacy**, **AdGuard**, **Peter Lowe**, **abuse.ch (URLhaus)** and **HaGeZi** for the filter lists.
- **Dark Reader** (MIT) and **Fuse.js** (Apache-2.0).
- **The Rust community** and every crate author we depend on. The full list is in [`NOTICE.md`](NOTICE.md), [`third_party/`](third_party/) and [`docs/licenses.md`](docs/licenses.md).

## License

AgentGo is released under the **Business Source License 1.1** (see [`LICENSE`](LICENSE)). You may use, copy, modify and redistribute it for non-commercial, evaluation, development, research and personal purposes. Production use by a commercial entity or for commercial purposes needs a commercial license from the author. On **2030-01-01** (or four years after the first public release of a version, whichever comes first) each version converts to the **Apache License 2.0**. Third-party components keep their own licenses.