# AgentGo Local Control Protocol

Status: Milestones 1-2 runtime foundation. The envelope, framing, error-code,
and local stdio contracts are implemented and tested; authenticated named
pipe control remains a Milestone 3 deliverable.

## Envelopes

Newline-delimited JSON, one document per line, 4 MiB frame cap
(`agentgo-protocol::framing`).

```json
{"v":1,"id":7,"method":"tab.navigate","params":{"tabId":"t-1","url":"https://www.google.com/search?q=example"}}
{"v":1,"id":7,"ok":true,"result":{"tabId":"t-1","loading":true}}
{"v":1,"id":8,"ok":false,"error":{"code":"policy.denied","message":"money actions require user approval"}}
{"v":1,"id":0,"event":"tab.statusChanged","params":{"tabId":"t-1","loading":false}}
```

- `v` is the envelope shape version (currently 1).
- `method` names are dotted, lowercase, and must exist in the capability
  manifest.
- Responses echo the request id. Events carry id `0`.

## Error codes

`invalid.envelope`, `method.notFound`, `invalid.params`,
`capability.unsupported`, `policy.denied`, `approval.required`,
`handoff.required`, `target.stale`, `timeout`, `recovery.exhausted`,
`unauthorized`, `idempotency.duplicate`, `internal`.

`capability.unsupported` is the honest answer for unimplemented
capabilities. No transport, adapter, or UI may substitute a fake success.

## Transports

1. **stdio (implemented)** — the agent host launches the executable and exchanges
  newline-delimited JSON on stdin/stdout.
2. **Windows named pipe (planned, Milestone 3)** — attach to a running instance; pipe ACL is
  restricted to the current user SID; a random 256-bit instance secret
  plus a one-time bootstrap token gate the session.
3. No TCP by default.

## Feature negotiation (planned, Milestone 3)

Handshake exchanges protocol versions and feature flags
(`agentgo-protocol::version::PROTOCOL_FEATURES`), e.g.
`transport.stdio`, `shell.bridge`.

## Adapters

- **Shell bridge** (`agentgo://shell` only): same envelope over the CEF
  message router; the method registry is
  `agentgo-shell-bridge::methods`.
- **MCP** (Milestone 9): tool `agentgo_<command_with_underscores>` per
  protocol command; mapping validated 1:1 with no privilege changes
  (`agentgo-mcp`).
- **SDKs** (Rust/TypeScript/Python, Milestone 9): generated from the
  same schemas.
