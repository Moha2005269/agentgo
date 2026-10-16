# ADR-0004: Local storage — %LOCALAPPDATA% layout, atomic writes, SQLite WAL

- Status: Accepted
- Date: 2026-09-07

## Context

AgentGo keeps settings, history, bookmarks, workflow metadata, audit
indexes, and resumable checkpoints locally. Data must survive crashes,
migrate forward safely, and never place plaintext secrets on disk.

## Decision

- Canonical layout under `%LOCALAPPDATA%\AgentGo\` (config, data,
  profiles, instances, artifacts, downloads, traces, crashes, logs) —
  implemented and tested in `crates/agentgo-storage::paths`.
- **Milestone 0 (now):** an atomic JSON state store with schema versions
  and pure-function migrations (`crates/agentgo-storage::store`) —
  temp-file + rename + fsync writes; corrupt or future-schema files are
  rejected loudly, never silently reset.
- **Milestone 2 (current foundation):** SQLite in WAL mode for history,
  bookmarks, workflow metadata, and audit indexes. Settings remain in the
  versioned atomic JSON store until their schema is migrated without changing
  the public settings contract. Migration discipline stays identical.
- Per-profile CEF request contexts/cache directories under
  `profiles\<profile_id>\`.
- Atomic writes everywhere; retention policies and complete local
  deletion/export.

## Consequences

- The file store remains an honest settings/tooling store; browser history and
  shared record tables use a real SQLite database rather than a fake JSON
  database.
- No plaintext credentials in any store (see ADR-0005).
