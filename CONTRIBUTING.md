# Contributing to AgentGo

## Ground rules

1. Work milestone by milestone; never land uncontrolled mega-patches.
2. Never replace behavior with mocks, timers, hard-coded data, or
   success toasts. Unsupported controls must be visibly disabled with an
   honest reason.
3. Keep implementation, schemas, tests, documentation, the capability
   manifest, and the bundled skill synchronized — the lockstep tests
   enforce this.
4. A capability flips to `implemented` in the manifest only in the same
   commit that lands its handler and its test id.
5. Policy changes require adversarial test coverage
   (`tests/policy/adversarial-plan.md`).
6. Do not perform purchases, publish releases, buy/sign certificates,
   or enable cloud services without explicit owner authority.

## Workflow

```powershell
scripts\fmt.ps1 -Fix     # format
scripts\lint.ps1         # clippy, warnings denied
scripts\test.ps1         # unit + integration + conformance
scripts\doctor.ps1       # environment check
```

Commit messages cite the capability ids and test ids touched. ADRs are
added for any decision that changes the architecture, contracts, or
pins.
