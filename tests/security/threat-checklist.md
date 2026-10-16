# Security tests

Milestone 0 status: trust-boundary predicates and redaction are unit
tested (`agentgo-shell-bridge::origin`, `agentgo-artifacts::redact`,
`agentgo-storage::paths` traversal guards, `agentgo-protocol::framing`
bounds). The release-level security gate (Milestone 11) adds protocol
fuzzing, sandbox validation, secret scans, dependency/license audit,
crash recovery, and the checklist below.

## Release checklist (M11)

- [ ] Protocol parser fuzzing (envelope, framing, schema boundaries).
- [ ] CEF sandbox validation (renderer/gpu/utility confined).
- [ ] Secret scan of sources, logs, and artifacts.
- [ ] Dependency + license audit (cargo audit/deny).
- [ ] Prompt-injection adversarial suite green.
- [ ] Money-action adversarial suite green.
- [ ] CAPTCHA never-solve suite green.
- [ ] Named-pipe ACL/secret/token attack suite green.
- [ ] Crash recovery (renderer kill, GPU kill, browser kill).
- [ ] Data deletion/export completeness.
- [ ] Reproducible build record.
