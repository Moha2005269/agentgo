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
| `browser.update_check` / `update_stage` | The address in the `updateManifest` setting (https, or loopback for tests) | Looks for a newer AgentGo; the manifest must be signed | Empty setting means no call |
| Site accounts you connect (Google, GitHub, Yandex, LinkedIn) | That provider's OAuth and API hosts | Token exchange and API calls you requested | Don't connect an account |
| Ad blocker | None at run time. The filter lists ship inside the app | Blocking happens locally | Settings, Ad blocker |
| Downloads | The URL you download | The file | `--allow-downloads` is required |
| Defender scan | None. `MpCmdRun.exe` scans locally | Safety | n/a |

Chromium's own background traffic is switched off in the packaged build:
component updates, background networking, sync, field-trial pings and domain
reliability reports (`disable-background-networking`, `disable-component-update`,
`disable-sync`, `no-pings`, `disable-domain-reliability`).

Build time only (not part of the app): `scripts/bootstrap.ps1` downloads the
pinned CEF distribution, and
`scripts/update-adblock-lists.ps1` refreshes the filter lists. Each checks the
hashes recorded in the repository.
