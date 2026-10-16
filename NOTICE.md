# AgentGo NOTICE

AgentGo Browser — an agent-first Chromium/CEF browser for Windows.

Copyright (c) 2026 Mhmad hassn.
Licensed under the Business Source License 1.1 (see LICENSE). Third-party components below keep their own licences.

## Bundled / linked third-party components

| Component | Version / pin | License | Use |
|---|---|---|---|
| Chromium Embedded Framework (binary distribution) | 152.0.5+gb129680+chromium-152.0.7977.54 (Windows x64 standard) | BSD-style (see third_party/licenses/CEF-LICENSE.txt and third_party/licenses/CHROMIUM-LICENSE.txt) | Web content rendering engine and embedding API |
| cef-rs (Rust CEF bindings) | git rev 83640144407b5742356b3ea6678a8b2de921e51d (152.0.0+152.0.5) | MIT OR Apache-2.0 | Rust bindings for the CEF C/C++ API |
| Rust toolchain | 1.98.1 | MIT/Apache-2.0 (dual) | Build toolchain (not redistributed) |

Chromium and CEF include additional third-party libraries; their notices ship
inside the CEF binary distribution (`LICENSE.txt`, `LICENSE.txt.chromium`) and
must accompany any redistributed AgentGo binaries.

## Trademarks

"AgentGo" naming, the wing/halo emblem, and product identifiers are reserved
by the project owner. Google is a trademark of Google LLC; AgentGo navigates
to Google Search but is not affiliated with or endorsed by Google LLC.

## Research-only references

The repositories under `work/agent-browser-sources/` (outside this repository)
are read-only research references and are not linked, bundled, or redistributed
with AgentGo. See third_party/README.md for the extraction map and rules.
