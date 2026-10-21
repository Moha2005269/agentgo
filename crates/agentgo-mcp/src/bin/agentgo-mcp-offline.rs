//! AgentGo's MCP server with no browser behind it.
//!
//! The browser runs on Windows 11 only. This binary builds anywhere and
//! answers MCP introspection (`initialize`, `tools/list`) with the real tool
//! list, so directories can check the server; tool calls say where to get
//! the browser.

use agentgo_mcp::client::Client;
use agentgo_mcp::server::serve;

const REASON: &str = "this is AgentGo's offline MCP server: it lists the tools but has no browser behind it. AgentGo runs on Windows 11: install it from https://github.com/Moha2005269/agentgo/releases/latest and connect `agentgo mcp`";

fn main() {
    let stdin = std::io::stdin();
    serve(Client::offline(REASON), stdin.lock(), std::io::stdout()).close();
}
