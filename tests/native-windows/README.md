# Native Windows lane tests

Milestone 8 scope: upload/download/save/print/native-dialog matrix —
scoped UI Automation for AgentGo-owned windows and selected native
dialogs only, with proof evidence. Fixture plan: native file picker on
the fixture page (`tests/fixtures/pages/fixture.html`), print dialog,
permission prompt. Out-of-scope attempts (arbitrary desktop windows)
must be refused by the lane (`agentgo-native-win::NativeError::OutOfScope`).
