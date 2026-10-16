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
| Ad blocker blocks and still lets sites work | `adblock_e2e`, `adblock_live_e2e` and the public tester pages (see `docs/issues-log.md`) |

Not covered by automation: Alt+Tab and taskbar behaviour on a real desktop,
and Authenticode signing.
