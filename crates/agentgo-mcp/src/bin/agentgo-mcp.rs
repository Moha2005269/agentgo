//! AgentGo's MCP server on stdio.
//!
//! Attaches to the newest running AgentGo browser, or starts `agentgo` from
//! PATH (set `AGENTGO_EXE` to use another path) when none is running.

use agentgo_mcp::client::Client;
use agentgo_mcp::server::serve;

fn main() {
    let client = Client::attach(None, "mcp").or_else(|_| {
        let exe = std::env::var_os("AGENTGO_EXE").unwrap_or_else(|| "agentgo".into());
        Client::launch(
            std::path::Path::new(&exe),
            &["--url".into(), "about:blank".into()],
        )
    });
    match client {
        Ok(client) => {
            let stdin = std::io::stdin();
            serve(client, stdin.lock(), std::io::stdout()).close();
        }
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}
