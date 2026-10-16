# ADR-0005: Secrets — Windows Credential Manager/DPAPI, universal redaction

- Status: Accepted
- Date: 2026-09-07

## Context

AgentGo will hold login sessions, optional vault credentials, and logs
that could contain secret material. The operator's profile and cookies
are sensitive and stay local.

## Decision

- Secrets live in Windows Credential Manager (DPAPI-protected) — no
  custom cross-device vault in v1.
- Credential autofill and credential visibility are **separate**
  permissions. Autofill can operate without exposing plaintext.
  Plaintext visibility is an advanced local-only setting, off by
  default, and still redacted from logs, screenshots, artifacts, and
  telemetry.
- Instance secrets (256-bit) are generated per run
  (`crates/agentgo-session::identity`), never logged, `Debug`-redacted.
- Redaction runs **before** persistence or emission:
  `crates/agentgo-artifacts::redact` covers authorization/cookie
  headers, named secret fields in JSON, and inline `key=value`
  assignments in text, with shape preserved.
- Traces/artifacts/har records redact authorization headers, cookies,
  tokens, and personal data.

## Consequences

- Redaction utilities are tested from day one (Milestone 0).
- DPAPI integration lands in Milestone 7 with the policy/approval work.
