# ADR-0008: Rust host on pinned cef-rs; C++ only as a documented shim

- Status: Accepted
- Date: 2026-09-07

## Context

The master plan requires a Rust-native control core (policy, protocol,
state, workflow, storage, artifacts, Windows lane) and permits a minimal
C++ shim only where CEF bindings are missing or unsafe.

## Decision

- The host is Rust. The workspace pins `cef-rs` to the exact git revision
  `83640144407b5742356b3ea6678a8b2de921e51d` (crate version
  `152.0.0+152.0.5`) in `Cargo.toml`; `crates/agentgo-cef` holds the same
  pins as code constants, and `scripts/pins.json` carries all three in
  lockstep (enforced by tests).
- The CEF C++ wrapper (`libcef_dll_wrapper`) is compiled by the cef-rs
  sys build with CMake+Ninja+MSVC against the pinned binary
  distribution — this is the only C++ compile in the default build.
- If a required CEF feature is missing or unsafe in the bindings, add a
  **very small** official-API C++ shim behind a documented C ABI in
  `crates/agentgo-cef`; never spread C++ across the core.
- The Windows sandbox uses CEF's `bootstrap.exe` launcher pattern with
  the app built as a cdylib (cefsimple model) — the bundling tooling
  comes from the pinned cef-rs revision.

## Consequences

- Milestone 0 builds do not compile any C++ (the `cef` dependency is
  feature-gated behind `runtime`); the first C++ wrapper build happens in
  the Milestone 1 spike.
- Upgrading the pin is an explicit, tested dependency PR that updates
  `Cargo.toml`, `pins.rs`, `pins.json`, and the distribution hashes
  together.
