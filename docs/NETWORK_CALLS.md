# Network calls AgentGo makes by itself

AgentGo has no telemetry, no analytics, no crash upload and no model calls.
Apart from the pages you or your agent open, these are the only connections it
can make, and each one is tied to something you asked for.

| When | Destination | Why | Can it be off? |
| --- | --- | --- | --- |
| You type a search in the address bar | Google, Bing or DuckDuckGo (your Settings choice) | The search itself | Pick the engine in Settings |
| You open the New Tab page | Shortcuts to Google Mail, Maps and Docs are plain links; nothing loads until you click | none | n/a |
| Menu, Translate page | `translate.google.com` | Translates the open page | Only when clicked |
| Extensions, Install from the Web Store | `clients2.google.com` (CRX download), `chromewebstore.google.com` (page link) | Fetches the package; the signature is verified before unpacking | Only when you install |
| Shortly after start and every 6 hours, `browser.update_check` / `update_stage` | `github.com/Moha2005269/agentgo/releases/latest/download/update-manifest.json` (or the `updateManifest` setting), then that release's zip | Looks for a newer AgentGo; the manifest must carry the release signature and the zip must match its SHA-256 | Installed copies only. "Update automatically" off still checks and shows a button; the person installs it |
| Site accounts you connect (Google, GitHub, Yandex, LinkedIn) | That provider's OAuth and API hosts | Token exchange and API calls you requested | Don't connect an account |
| Ad blocker | None at run time. The filter lists ship inside the app | Blocking happens locally | Settings, Ad blocker |
| Downloads | The URL you download | The file | `--allow-downloads` is required |
| Defender scan | None. `MpCmdRun.exe` scans locally | Safety | n/a |
| Import, "Open Chrome export" | None. It starts your installed Chrome on its own password page | You asked to export | Only when clicked |

AgentGo Setup (the installer, not the browser): `AgentGo-Setup.exe` asks
`api.github.com` for the latest release and downloads its zip and `.sha256`
from `github.com`; the zip must match before anything is unpacked.
`AgentGo-Setup-full.exe` carries the package inside and makes no connection.

Chromium's own background traffic is switched off in the packaged build:
component updates, background networking, sync, field-trial pings and domain
reliability reports (`disable-background-networking`, `disable-component-update`,
`disable-sync`, `no-pings`, `disable-domain-reliability`).

Build time only (not part of the app): `scripts/bootstrap.ps1` downloads the
pinned CEF distribution, and
`scripts/update-adblock-lists.ps1` refreshes the filter lists. Each checks the
hashes recorded in the repository.
