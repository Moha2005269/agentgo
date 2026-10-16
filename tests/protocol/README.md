# Protocol conformance tests

`scripts/test-protocol-conformance.ps1` is the harness. Milestone 0
scope: schema parse checks, manifest structural checks, and the binary's
pinned version report. Milestone 3 extends this directory with live
stdio request/response session fixtures (handshake, method not found,
invalid params, policy denied, capability unsupported, backpressure,
cancellation, idempotency) driven against a real launched instance.
