//! Protocol client used by the CLI and the MCP server.
//!
//! Two ways to reach a browser, both speaking the same newline-delimited
//! JSON protocol: attach to a running instance over its authenticated named
//! pipe, or launch a private instance as a child process over stdio.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use serde_json::{json, Value};

/// Client failure, already phrased for an agent or a terminal.
#[derive(Debug, Clone, thiserror::Error)]
pub enum ClientError {
    /// No reachable browser instance.
    #[error("{0}")]
    Unavailable(String),
    /// Transport broke mid-request.
    #[error("connection to AgentGo was lost: {0}")]
    Io(String),
    /// The browser answered with a typed protocol error.
    #[error("{code}: {message}")]
    Protocol {
        /// Stable error code, e.g. `target.stale`.
        code: String,
        /// Message with a recovery hint.
        message: String,
        /// Structured detail such as an approval id.
        data: Value,
    },
}

/// A connected protocol client.
pub struct Client {
    reader: Box<dyn BufRead + Send>,
    writer: Box<dyn Write + Send>,
    child: Option<Child>,
    next_id: u64,
}

/// AgentGo's local data root (`AGENTGO_DATA_ROOT` or `%LOCALAPPDATA%\AgentGo`).
pub fn data_root() -> Option<PathBuf> {
    std::env::var_os("AGENTGO_DATA_ROOT")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("LOCALAPPDATA").map(|dir| PathBuf::from(dir).join("AgentGo")))
}

/// Running instances, newest first, as `(instance id, token)`. An instance
/// counts as running while its token file exists (removed at clean exit).
pub fn running_instances(root: &Path) -> Vec<(String, String)> {
    let mut found: Vec<(std::time::SystemTime, String, String)> =
        std::fs::read_dir(root.join("instances"))
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| {
                let token_path = entry.path().join("token");
                let token = std::fs::read_to_string(&token_path).ok()?;
                let modified = std::fs::metadata(&token_path).ok()?.modified().ok()?;
                Some((
                    modified,
                    entry.file_name().to_string_lossy().into_owned(),
                    token.trim().to_string(),
                ))
            })
            .collect();
    found.sort_by_key(|entry| std::cmp::Reverse(entry.0));
    found
        .into_iter()
        .map(|(_, id, token)| (id, token))
        .collect()
}

impl Client {
    /// Attaches to a running instance (the newest, or the one named) over
    /// its named pipe and authenticates with the instance token.
    pub fn attach(instance: Option<&str>, client_name: &str) -> Result<Self, ClientError> {
        let root = data_root().ok_or_else(|| {
            ClientError::Unavailable("cannot locate the AgentGo data directory".into())
        })?;
        let candidates: Vec<_> = running_instances(&root)
            .into_iter()
            .filter(|(id, _)| instance.is_none_or(|wanted| wanted == id))
            .collect();
        for (id, token) in candidates {
            let pipe = format!(r"\\.\pipe\agentgo-{id}");
            // A stale token from a crashed instance simply fails to open.
            let Ok(file) = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&pipe)
            else {
                continue;
            };
            let Ok(writer) = file.try_clone() else {
                continue;
            };
            let mut client = Self {
                reader: Box::new(BufReader::new(file)),
                writer: Box::new(writer),
                child: None,
                next_id: 1,
            };
            client.call(
                "session.hello",
                json!({"token": token, "client": client_name}),
            )?;
            return Ok(client);
        }
        Err(ClientError::Unavailable(
            "no running AgentGo browser found. Start AgentGo first, or use `agentgo mcp` / `agentgo run --stdio` to launch one".into(),
        ))
    }

    /// Launches a private browser instance as a child process over stdio.
    pub fn launch(executable: &Path, extra_args: &[String]) -> Result<Self, ClientError> {
        let mut child = Command::new(executable)
            .arg("run")
            .arg("--stdio")
            .args(extra_args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // Browser diagnostics must never reach an MCP host's protocol stream.
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| {
                ClientError::Unavailable(format!(
                    "could not start {}: {error}",
                    executable.display()
                ))
            })?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| ClientError::Io("no stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| ClientError::Io("no stdout".into()))?;
        Ok(Self {
            reader: Box::new(BufReader::new(stdout)),
            writer: Box::new(stdin),
            child: Some(child),
            next_id: 1,
        })
    }

    /// Sends one request and waits for its response, skipping events.
    pub fn call(&mut self, method: &str, params: Value) -> Result<Value, ClientError> {
        let id = self.next_id;
        self.next_id += 1;
        let line = json!({"v": 1, "id": id, "method": method, "params": params}).to_string();
        self.writer
            .write_all(format!("{line}\n").as_bytes())
            .and_then(|_| self.writer.flush())
            .map_err(|error| ClientError::Io(error.to_string()))?;
        loop {
            let mut response = String::new();
            let read = self
                .reader
                .read_line(&mut response)
                .map_err(|error| ClientError::Io(error.to_string()))?;
            if read == 0 {
                return Err(ClientError::Io("the browser closed the connection".into()));
            }
            let Ok(message) = serde_json::from_str::<Value>(&response) else {
                continue;
            };
            if message["id"].as_u64() != Some(id) {
                continue; // an event or a stale reply
            }
            if message["ok"] == true {
                return Ok(message["result"].clone());
            }
            return Err(ClientError::Protocol {
                code: message["error"]["code"]
                    .as_str()
                    .unwrap_or("internal")
                    .to_string(),
                message: message["error"]["message"]
                    .as_str()
                    .unwrap_or("unknown error")
                    .to_string(),
                data: message["error"]["data"].clone(),
            });
        }
    }

    /// Closes a browser this client launched; attached browsers stay open.
    pub fn close(mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = self.call("browser.shutdown", json!({}));
            for _ in 0..50 {
                if matches!(child.try_wait(), Ok(Some(_))) {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            let _ = child.kill();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instances_are_listed_newest_first_and_need_a_token() {
        let root = std::env::temp_dir().join(format!("agentgo-client-test-{}", std::process::id()));
        for (name, token) in [
            ("i-old", Some("aa")),
            ("i-new", Some("bb")),
            ("i-stopped", None),
        ] {
            let dir = root.join("instances").join(name);
            std::fs::create_dir_all(&dir).unwrap();
            if let Some(token) = token {
                std::fs::write(dir.join("token"), token).unwrap();
                std::thread::sleep(std::time::Duration::from_millis(30));
            }
        }
        let found = running_instances(&root);
        assert_eq!(
            found,
            vec![
                ("i-new".to_string(), "bb".to_string()),
                ("i-old".to_string(), "aa".to_string())
            ]
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
