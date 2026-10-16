# AgentGo Threat Model (initial)

Status: Milestone 0 baseline. Refined at Milestones 3 (protocol), 7
(policy/secrets), and 11 (release security).

## Assets

- Operator profiles, cookies, sessions, credentials (DPAPI vault).
- Local data: history, settings, artifacts, traces, audit logs.
- The trusted shell bridge (native RPC) and the policy engine.
- Machine integrity (downloads, native lane, subprocess spawning).

## Trust boundaries

1. Trusted `agentgo://shell` origin ⟷ untrusted web content.
2. Local protocol clients ⟷ AgentGo instance (named-pipe ACL + instance
   secret + bootstrap token).
3. Agent commands ⟷ policy engine ⟷ execution (policy is below every
   adapter).
4. Downloads/artifacts ⟷ filesystem scopes.

## Threats and mitigations

| # | Threat | Mitigation | Status |
|---|---|---|---|
| T1 | Malicious page calls the native bridge | Bridge bound to exact `agentgo://shell` origin/main frame; predicate tested | designed + tested predicate; runtime enforcement M1 |
| T2 | Look-alike origin tricks (`agentgo://shell.evil`) | Exact-host, no-userinfo, no-port matching | tested |
| T3 | Prompt injection from page text | Policy only from operator/protocol; page content never grants permissions | designed; adversarial suite M7 |
| T4 | Money-action bypass through any adapter | Single decision matrix below all adapters; final submit is money-class | implemented + tested |
| T5 | CAPTCHA bypass attempt | Handoff is mandatory in every mode; no solve/outsource code paths exist | implemented in matrix |
| T6 | Cross-instance agent control | Per-instance 256-bit secrets, user-SID pipe ACL, unique instance ids | identity generation implemented + tested; transports M3 |
| T7 | Secret leakage into logs/traces | Redaction-before-persistence utilities; Debug-redacted secrets | implemented + tested |
| T8 | Path traversal via ids/paths | Segment sanitization (traversal/reserved names); file scopes reject `..` | implemented + tested |
| T9 | Unbounded memory via protocol frames | 4 MiB frame cap with loud errors | implemented + tested |
| T10 | Corrupt/future-schema state files | Loud rejection, never silent reset | implemented + tested |
| T11 | Compromised CEF binary supply chain | Pinned URL + size + SHA-1 + SHA-256 verification before extraction | implemented (bootstrap) |
| T12 | Downloaded file auto-execution | Downloads disabled by default; execution is a separate approval row | matrix implemented; pipeline M8 |
| T13 | Renderer compromise | CEF process isolation plus termination/unresponsive diagnostics; official bootstrap package passes the sandboxed startup smoke while direct sandbox invocation fails closed | foundation verified; broader release confinement evidence remains M11 |
| T14 | Fake UI success (toast without action) | Lockstep tests + "no implemented status without tests" manifest rule | implemented + tested |

## Out of scope (v1)

- Anti-bot evasion of any kind (explicitly never).
- Cloud sync/telemetry (default-off local-only product).
- DRM circumvention.
