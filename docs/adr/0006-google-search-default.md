# ADR-0006: Google is the default and primary search provider

- Status: Accepted
- Date: 2026-09-07

## Context

The approved UI prototype ships an "AgentGo Search" option and a
default. The product decision (master plan section 2.5) removes AgentGo
Search entirely: a plain query navigates the active tab to
`https://www.google.com/search?q=...`. AgentGo does not proxy search and
does not provide a hosted search backend.

## Decision

- Google is the default and primary search provider.
- Omnibox/plain-query resolution maps to
  `https://www.google.com/search?q=<query>`; URLs navigate directly.
- "AgentGo Search" is removed from the product: the settings option, the
  persisted default, and any residual UI references. Existing stored
  values migrate to Google (Milestone 2 shell migration).
- Alternate search providers are an optional setting only after Google
  works; AgentGo Search is never reintroduced.

## Consequences

- The bundled skill documents the Google URL and contains no
  "AgentGo Search" reference (enforced by the lockstep test).
- The Milestone 0 lockstep test pins the migration obligation on
  `ui/index.html` (the approved prototype still contains the option) so
  Milestone 2 cannot silently forget it.
- No network search dependency exists in the shell at Milestone 0; the
  shell loads no search backend at all.
