# AgentGo SDKs

Small clients for AgentGo's local HTTP and WebSocket transport. The person
turns the transport on (`sdk.http {on: true}` from the browser itself); it
listens on 127.0.0.1 only and every call needs the bearer token from the
instance's `http-token` file (`sdk.http` names the file).

| Folder | Language | Entry |
|---|---|---|
| `typescript/` | Node 22+ | `agentgo.mjs` |
| `python/` | Python 3.9+, standard library only | `agentgo.py` |
| `rust/` | Rust, standard library and serde_json | `src/lib.rs` |
| `plugin/` | A template local skill (`skill.install_local`) | `template/skill.json` |

Wire format: `POST /v1/call` with `{"method", "params"}` returns the
protocol reply `{"v", "id", "ok", "result" | "error"}`. `GET /v1/ws` is a
WebSocket that takes protocol request lines (`{"v":1,"id","method","params"}`)
and sends the replies and subscribed events. Agents over stdio, the named
pipe or MCP do not need these clients.

`tests/integration/sdk_e2e.js` drives all three clients against a real browser.
