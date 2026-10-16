# AgentGo for Rust

Standard library and `serde_json` only.

```rust
let ag = agentgo::Client::from_token_file(port, token_file)?;
let page = ag.call("navigate.url", serde_json::json!({"url": "https://example.com"}))?;
```

`cargo run --example call -- <port> <token file> browser.version` tries it.
