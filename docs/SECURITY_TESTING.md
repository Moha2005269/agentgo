# How the security claims are tested

Run the whole suite on a private desktop (nothing appears on your screen):

```powershell
powershell -File scripts\build.ps1 -Runtime              # tools + dev exe
powershell -File scripts\build.ps1 -Runtime -Sandbox     # the shipped shape
$env:AGENTGO_EXE = "$PWD\target\agentgo-sandbox\debug\agentgo.exe"
node scripts\dev\run-integration.js
```

Pointing `AGENTGO_EXE` at the sandboxed bundle makes every test exercise the
same renderer sandbox the package ships.

| Claim | Test |
| --- | --- |
| Agents cannot open `file://` (any letter case) | `tests/integration/security_e2e.js` (1) |
| A content tab cannot be sent to `agentgo://shell/...` | `security_e2e.js` (2, 3) |
| SDK server refuses rebinding `Host` and foreign `Origin` | `security_e2e.js` (4), unit test `only_loopback_hosts_and_origins_pass` |
| Unsigned update manifests are ignored | `security_e2e.js` (5), unit test `manifest_signature_is_enforced` |
| CRX packages need a valid signature for their id | unit tests in `runtime/pack.rs` (signed CRX3 with ECDSA, tampering, garbage) |
| Zip extraction cannot escape its folder | unit tests `zip_slip_is_refused`, `zip_extracts_normal_files` |
| Downloads are marked and scanned; flagged files do not open | `tests/integration/safety_reader_e2e.js`, `runtime/scan.rs` unit test |
| Renderers run sandboxed | Check by hand: start the bundle, then read each `agentgo.exe --type=renderer` token with Process Explorer; integrity must be Untrusted |
| Fingerprint: hooks read as native code, blank iframes protected, per-site noise for the person, one stable fingerprint for an agent that differs from the person's and survives restarts | `tests/integration/fingerprint_e2e.js` |
| Fingerprint protection on the first page of a tab and across mode switches | `security_live_e2e.js` (fingerprint checks), D80 in `docs/issues-log.md` |
| Certificate errors are never bypassed (expired, self-signed, wrong host, untrusted root, revoked) | `tests/integration/security_live_e2e.js` (badssl.com, live) |
| Known malware and phishing sites are stopped before the request; look-alike addresses are recognised; agents cannot open a blocked page, even after disconnecting | `security_live_e2e.js` (live), unit tests in `crates/agentgo-adblock/src/threats.rs` (homographs, brand subdomains, brand+lure, brands' own and country domains never flagged); survey over the Tranco top 100,000 with `AGENTGO_DOMAINS=... cargo test -p agentgo-adblock -- --ignored survey` (0 look-alike false positives) |
| Tracking parameters removed, `Sec-GPC: 1` sent, third-party cookies blocked, http upgraded, notification requests refused | `security_live_e2e.js` (httpbin.org/headers for the header), `safesite.rs` unit test for the parameters |
| WebRTC reveals no private or extra addresses | `security_live_e2e.js` (only a server-reflexive candidate) |
| Installer: package checked against its SHA-256 and every file against `SHA256SUMS.txt`; zip entries cannot leave the install folder | `installer/src/main.rs` (`install`, `extract`, `check_sums`) |
| Ad blocker blocks and still lets sites work | `adblock_e2e`, `adblock_live_e2e` and the public tester pages (see `docs/issues-log.md`) |

Live public testers (`node scripts\dev
un-integration.js --live security_live_e2e`,
report in `target/security-live-report.json`), last run 2026-10-17: badssl
5/5 blocked, WebRTC one `srflx` candidate only, adblock-tester.com 100/100,
EFF Cover Your Tracks "strong protection against Web tracking", listed malware
and phishing sites and a PayPal look-alike blocked before loading, GPC header
seen by httpbin.org, tracking parameters removed, http upgraded.

Not covered by automation: Alt+Tab and taskbar behaviour on a real desktop,
and Authenticode signing.
