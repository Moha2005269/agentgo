# Third-party extraction map and provenance rules

AgentGo is built in this repository. The research archive at
`work/agent-browser-sources/` (outside this repo) is **read-only
reference** and is never linked, bundled, or redistributed.

## Rules

1. Do not add research repositories as runtime dependencies.
2. Port small, tested concepts behind AgentGo-owned interfaces.
3. Before borrowing any code, record in the table below: origin file,
   commit, license, modification, and destination.
4. Reimplement concepts when license/provenance is unclear.

## Borrowed so far

| Origin | Commit | License | What | Modification | Destination |
|---|---|---|---|---|---|
| chromiumembedded/cef (binary distribution) | 152.0.5+gb129680+chromium-152.0.7977.54 | BSD-style (see licenses/CEF-LICENSE.txt) | libcef.dll and runtime, headers, wrapper sources | none (pinned binary; hash-verified by scripts/bootstrap.ps1) | third_party/cef-dist (gitignored, downloaded) |
| tauri-apps/cef-rs | 83640144407b5742356b3ea6678a8b2de921e51d | MIT OR Apache-2.0 | Rust CEF bindings + sys build + bundling tooling | none (pinned git dependency) | crates/agentgo-cef (feature `runtime`), build tooling |
| brave/adblock-rust (crate `adblock`) | 0.13.3 (crates.io) | MPL-2.0 | Filter engine that matches requests against EasyList syntax | none (unmodified crate dependency, default `single-thread` feature off) | crates/agentgo-adblock |
| EasyList and EasyPrivacy (easylist.to) | snapshot dated in the `! Version:` line of each file | GPLv3+ or CC BY-SA 3.0 (see licenses/EasyList-NOTICE.txt) | Ad and tracker filter lists | none (unmodified snapshots; refresh with scripts/update-adblock-lists.ps1) | crates/agentgo-adblock/data/lists, compiled into the binary |
| uBlock Origin filter lists (uAssets: filters, privacy, badware, unbreak, quick-fixes) | snapshot dated in each list header | GPLv3 | Ad, tracker, malware and site-fix filters, plus scriptlets | none (unmodified snapshots) | crates/agentgo-adblock/data/lists |
| AdGuard Base (without EasyList), Social Media and DNS filters | snapshot versions in each list header | GPLv3 (AdGuard filters) | Ad, social-widget and tracking-domain filters | none (unmodified snapshots) | crates/agentgo-adblock/data/lists |
| Peter Lowe's ad and tracking server list (pgl.yoyo.org) | snapshot | free for use, attribution | Ad and tracking domains | none | crates/agentgo-adblock/data/lists |
| URLhaus filter (malware-filter, abuse.ch data) | snapshot | CC0 / abuse.ch terms | Malicious URL blocklist | none | crates/agentgo-adblock/data/lists |
| HaGeZi Pro++ blocklist | snapshot | GPLv3 | Ad, tracker and telemetry domains | none (unmodified snapshot) | crates/agentgo-adblock/data/lists |
| Dark Reader | 4.9.133 | MIT (licenses/darkreader-LICENSE-MIT) | Recolours web pages for dark mode on websites | none (unmodified build) | crates/agentgo-cef/src/runtime/darkreader.js |
| Fuse.js | 7.5.0 | Apache-2.0 (licenses/fuse.js-LICENSE-Apache-2.0) | Fuzzy search in the command palette | none (wrapped as a global `Fuse`) | ui/vendor/fuse.min.js |
| AdGuard Cookie Notices and Mobile Ads filters; uBlock Origin Cookie notices; Fanboy Cookie Monster | snapshot versions in each list header | GPLv3 / CC BY 3.0 (Fanboy) | Cookie banner, consent pop-up and mobile ad filters | none (unmodified snapshots) | crates/agentgo-adblock/data/lists |
| agentgo-extra.txt | written for AgentGo | project licence | Third-party-only host blocks that public test pages found reachable | original | crates/agentgo-adblock/data/lists |

| brave/adblock-resources | resources.json snapshot | MPL-2.0 (uBlock Origin scriptlets are GPLv3) | Scriptlets and redirect resources for the filter engine | none (unmodified snapshot) | crates/agentgo-adblock/data/resources.json |
| Diniboy1123/usque | 4.2.1 (GitHub release, sha256 pinned in scripts/fetch-usque.ps1) | MIT (licenses/usque-LICENSE-MIT) | Unofficial open-source Cloudflare WARP (MASQUE) client, run as a separate local SOCKS5 proxy for the VPN switch | none (unmodified release binary; fetch with scripts/fetch-usque.ps1) | third_party/usque, shipped as usque.exe next to agentgo.exe |
| ring 0.17 | ISC-style (OpenSSL/BoringSSL lineage) | none (crates.io dependency) | Ed25519 and RSA/ECDSA verification for update manifests and extension packages | none | crates/agentgo-cef (feature `runtime`) |
| zip 2 | MIT | none (crates.io dependency) | Extracts updates and extension packages with path checks | none | crates/agentgo-cef (feature `runtime`) |

The build step (`examples/build_snapshot.rs`) drops the `@@` allow-rules of the AdGuard DNS list before compiling: they exist to keep DNS filtering from breaking apps and would undo blocks made by the other lists.

## Concept-extraction map (concepts only, no code)

- `cef`, `cef-project`, `cef-rs`: runtime lifecycle, Views/OSR, requests,
  process messages, downloads, DevTools, packaging.
- `browser-harness`, `browser-use`: session lifecycle, browser ownership,
  intervention, skills.
- `playwright-mcp`, `agent-browser`, `browser-mcp`: action semantics,
  selectors, uploads, dialogs, profiles, conformance ideas.
- `drisp-browser-mcp`, `bladebro`, `uiuing-browser-agent`,
  `malovlab-agent-browser`: compact state, stable ids, regions, diffs,
  re-resolution, verification, recovery.
- `browser-agent-native`: compact visual/semantic representation and
  token budgeting.
- `snact`: snap/action loop, re-snap, persistence, record/replay.
- `bap`, `graph-memory-browser-mcp`: protocol/schema/capability
  inspiration.
- `windows-harness`: UIA, native dialogs, input/focus truthfulness,
  proof images.
- `foundry-browser-use`, `i-am-not-a-robot`: intervention state machine
  and same-session human relay.
- `BrowserGym`: evaluation only.

## License files

- `licenses/CEF-LICENSE.txt` — CEF project license.
- `licenses/cef-rs-LICENSE-MIT`, `licenses/cef-rs-LICENSE-APACHE`.
- `licenses/adblock-rust-LICENSE-MPL-2.0`, `licenses/EasyList-NOTICE.txt`, `licenses/usque-LICENSE-MIT`
  (packaged into `licenses/` by `scripts/package.ps1`).
- The CEF binary distribution additionally carries `LICENSE.txt` and
  `LICENSE.txt.chromium` (Chromium and its third-party notices); those
  must accompany any redistributed AgentGo binaries.
