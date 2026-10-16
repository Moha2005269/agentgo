//! Client for AgentGo's local HTTP transport (`POST /v1/call`).
use std::io::{Read, Write};
use std::net::TcpStream;

use serde_json::{json, Value};

/// A failed call: the protocol error code, message and data.
#[derive(Debug)]
pub struct Error {
    pub code: String,
    pub message: String,
    pub data: Value,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for Error {}

fn fail(code: &str, message: impl Into<String>) -> Error {
    Error { code: code.into(), message: message.into(), data: Value::Null }
}

pub struct Client {
    port: u16,
    token: String,
    client: String,
}

impl Client {
    pub fn new(port: u16, token: impl Into<String>) -> Self {
        Self { port, token: token.into(), client: "rust".into() }
    }

    pub fn from_token_file(port: u16, path: impl AsRef<std::path::Path>) -> Result<Self, Error> {
        let token = std::fs::read_to_string(path).map_err(|e| fail("unauthorized", e.to_string()))?;
        Ok(Self::new(port, token.trim()))
    }

    /// Calls one command and returns its result.
    pub fn call(&self, method: &str, params: Value) -> Result<Value, Error> {
        let body = json!({"method": method, "params": params}).to_string();
        let mut stream = TcpStream::connect(("127.0.0.1", self.port)).map_err(|e| fail("internal", e.to_string()))?;
        write!(
            stream,
            "POST /v1/call HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {}\r\nX-AgentGo-Client: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            self.token,
            self.client,
            body.len()
        )
        .map_err(|e| fail("internal", e.to_string()))?;
        let mut text = String::new();
        stream.read_to_string(&mut text).map_err(|e| fail("internal", e.to_string()))?;
        let (head, body) = text.split_once("\r\n\r\n").ok_or_else(|| fail("internal", "no HTTP body"))?;
        if !head.starts_with("HTTP/1.1 200") {
            return Err(fail("unauthorized", head.lines().next().unwrap_or("")));
        }
        let reply: Value = serde_json::from_str(body).map_err(|e| fail("internal", e.to_string()))?;
        if reply["ok"] == true {
            Ok(reply["result"].clone())
        } else {
            let error = &reply["error"];
            Err(Error {
                code: error["code"].as_str().unwrap_or("internal").into(),
                message: error["message"].as_str().unwrap_or("").into(),
                data: error["data"].clone(),
            })
        }
    }
}
