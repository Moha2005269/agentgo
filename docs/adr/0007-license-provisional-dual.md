# ADR-0007: License — provisional dual MIT OR Apache-2.0

- Status: Provisional (owner must finalize before any public release)
- Date: 2026-09-07

## Context

The master plan (section 17.1) recommends Apache-2.0 or dual
MIT/Apache-2.0 and states the user must make the final legal choice
before public release. The implementation agent must not silently choose
legal licensing for the owner, but also must not block Milestone 0.

## Decision

Adopt **dual MIT OR Apache-2.0** as the recommended provisional default
(matching cef-rs and the wider Rust ecosystem, and compatible with the
CEF BSD-style license). LICENSE, LICENSE-MIT, and LICENSE-APACHE are in
the repository root. NOTICE.md records bundled components.

**The project owner must review and finalize this choice before any
public release, binary redistribution, or external contribution.** If
the owner chooses differently, the change is a root-only file swap plus
a workspace `license` field update.

## Consequences

- Nothing is published or redistributed by the implementation agent.
- CEF's BSD-style license and its bundled third-party notices must
  accompany any future binary distribution (see NOTICE.md).
