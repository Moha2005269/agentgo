# ADR-0003: Local control protocol — stdio JSON + Windows named pipe

- Status: Accepted
- Date: 2026-09-07

## Context

AgentGo is agent-agnostic: any host (Codex, Claude Code, OpenCode, a
plain CLI) must receive the same capabilities. A hosted web API would
violate the local-first, vendor-neutral requirement.

## Decision

- **Transports (P0):** newline-delimited JSON over stdio for launched
  instances; a Windows named pipe for attaching to a running instance.
- **No TCP port by default.** A later loopback transport is disabled by
  default and uses the same authentication and schemas.
- **Named pipe security:** ACL restricted to the current user SID, random
  256-bit instance secret, unique instance id, one-time bootstrap token
  exchanged for a short-lived client session.
- **Envelope:** versioned (`v`), correlation ids, typed dotted method
  names, typed error codes (see `crates/agentgo-protocol` and
  `schemas/protocol.schema.json`). Policy is evaluated below every
  adapter so CLI, MCP, SDK, shell, and replay cannot disagree.
- **MCP adapter:** a thin 1:1 mapping (`agentgo_<command>` tool names),
  no added or removed privileges (validated by tests in
  `crates/agentgo-mcp`).

## Consequences

- Framing, envelope, and error-code logic live in `agentgo-protocol`
  with unit tests today; transports land in Milestone 3.
- The shell bridge (trusted `agentgo://shell` origin only) speaks the
  same envelope through CEF's message router.
