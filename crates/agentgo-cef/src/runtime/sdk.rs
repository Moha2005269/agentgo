//! The localhost HTTP and WebSocket transport (`sdk.http`, `sdk.websocket`)
//! and the client packages in `sdk/` (`sdk.rust`, `typescript`, `python`,
//! `plugin`, `skill`, `test`).
//!
//! The server listens on 127.0.0.1 only, is off until the person turns it
//! on, and every request needs the bearer token written to the instance
//! folder (`http-token`), which only the person's Windows account can read.
//!   POST /v1/call  {"method", "params"}  ->  the protocol reply
//!   GET  /v1/ws    WebSocket: protocol request lines in, replies and events out
//! Calls run through the same dispatcher as every other transport.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicU16, AtomicUsize, Ordering};
use std::sync::mpsc;

use agentgo_protocol::{ErrorCode, Request};
use serde_json::{json, Value};

use super::sessions::{self, Caller, Fail, Reply};
use super::{dispatch, exec, state};

static PORT: AtomicU16 = AtomicU16::new(0);
static CONNECTIONS: AtomicUsize = AtomicUsize::new(0);

fn token_file() -> std::path::PathBuf {
    state::state()
        .paths
        .instance_dir(&state::state().instance_id)
        .join("http-token")
}

fn token() -> Option<String> {
    std::fs::read_to_string(token_file())
        .ok()
        .map(|t| t.trim().to_string())
}

fn same(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

/// SHA-1, only for the WebSocket handshake (RFC 6455 requires it).
fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];
    let mut message = data.to_vec();
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for chunk in message.chunks(64) {
        let mut w = [0u32; 80];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[4 * i],
                chunk[4 * i + 1],
                chunk[4 * i + 2],
                chunk[4 * i + 3],
            ]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, word) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A827999),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6),
            };
            let t = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*word);
            (e, d, c, b, a) = (d, c, b.rotate_left(30), a, t);
        }
        for (slot, value) in h.iter_mut().zip([a, b, c, d, e]) {
            *slot = slot.wrapping_add(value);
        }
    }
    let mut out = [0u8; 20];
    for (i, word) in h.iter().enumerate() {
        out[4 * i..4 * i + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

pub(super) fn base64(bytes: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    bytes
        .chunks(3)
        .flat_map(|c| {
            let n = (c[0] as u32) << 16
                | (*c.get(1).unwrap_or(&0) as u32) << 8
                | *c.get(2).unwrap_or(&0) as u32;
            (0..4).map(move |i| {
                if i <= c.len() {
                    T[(n >> (18 - 6 * i) & 63) as usize] as char
                } else {
                    '='
                }
            })
        })
        .collect()
}

fn respond(stream: &mut TcpStream, status: &str, body: &str) {
    let _ = write!(stream, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n{body}", body.len());
}

/// Runs one call on the UI thread and waits for its reply line.
fn call_on_ui(session: String, request: Request) -> String {
    let (tx, rx) = mpsc::channel();
    exec::post(move || {
        exec::spawn(async move {
            let caller = Caller { session };
            let id = request.id;
            let reply = dispatch::dispatch(&caller, &request.method, request.params).await;
            let _ = tx.send(dispatch::encode(id, reply));
        });
    });
    state::wake_ui();
    rx.recv_timeout(std::time::Duration::from_secs(600))
        .unwrap_or_else(|_| dispatch::encode(0, Err(Fail::new(ErrorCode::Timeout, "no reply"))))
}

fn ws_frame(text: &str) -> Vec<u8> {
    let payload = text.as_bytes();
    let mut frame = vec![0x81u8];
    match payload.len() {
        n if n < 126 => frame.push(n as u8),
        n if n < 65_536 => {
            frame.push(126);
            frame.extend_from_slice(&(n as u16).to_be_bytes());
        }
        n => {
            frame.push(127);
            frame.extend_from_slice(&(n as u64).to_be_bytes());
        }
    }
    frame.extend_from_slice(payload);
    frame
}

/// Reads one client frame: (opcode, payload).
fn ws_read(reader: &mut impl Read) -> Option<(u8, Vec<u8>)> {
    let mut head = [0u8; 2];
    reader.read_exact(&mut head).ok()?;
    let opcode = head[0] & 0x0F;
    let masked = head[1] & 0x80 != 0;
    let mut len = (head[1] & 0x7F) as u64;
    if len == 126 {
        let mut b = [0u8; 2];
        reader.read_exact(&mut b).ok()?;
        len = u16::from_be_bytes(b) as u64;
    } else if len == 127 {
        let mut b = [0u8; 8];
        reader.read_exact(&mut b).ok()?;
        len = u64::from_be_bytes(b);
    }
    if len > 16 * 1024 * 1024 || !masked {
        return None; // clients must mask; oversized frames drop the client
    }
    let mut mask = [0u8; 4];
    reader.read_exact(&mut mask).ok()?;
    let mut payload = vec![0u8; len as usize];
    reader.read_exact(&mut payload).ok()?;
    for (i, byte) in payload.iter_mut().enumerate() {
        *byte ^= mask[i % 4];
    }
    Some((opcode, payload))
}

/// `host[:port]` where host is exactly 127.0.0.1, localhost or [::1].
fn is_loopback_host(host: &str) -> bool {
    let name = if let Some(rest) = host.strip_prefix('[') {
        match rest.split_once(']') {
            Some((inner, tail)) if tail.is_empty() || tail.starts_with(':') => inner,
            _ => return false,
        }
    } else {
        host.split(':').next().unwrap_or("")
    };
    matches!(
        name.to_ascii_lowercase().as_str(),
        "127.0.0.1" | "localhost" | "::1"
    )
}

fn is_loopback_origin(origin: &str) -> bool {
    origin
        .strip_prefix("http://")
        .is_some_and(|rest| is_loopback_host(rest.trim_end_matches('/')))
}

fn serve(mut stream: TcpStream, number: usize) {
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(30)));
    let mut reader = BufReader::new(stream.try_clone().expect("stream"));
    let mut first = String::new();
    if reader.read_line(&mut first).is_err() {
        return;
    }
    let mut headers = std::collections::HashMap::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).is_err() || line.trim().is_empty() {
            break;
        }
        if headers.len() >= 64 || line.len() > 8192 {
            return respond(
                &mut stream,
                "431 Request Header Fields Too Large",
                r#"{"error":"headers too large"}"#,
            );
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }
    let mut parts = first.split_whitespace();
    let (verb, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    // Browsers attach an Origin header; web pages must never drive AgentGo.
    if headers
        .get("origin")
        .is_some_and(|o| !is_loopback_origin(o))
    {
        return respond(
            &mut stream,
            "403 Forbidden",
            r#"{"error":"cross-origin requests are refused"}"#,
        );
    }
    // A DNS-rebinding page reaches this port under its own name; only a
    // loopback Host header is served.
    if !headers.get("host").is_some_and(|h| is_loopback_host(h)) {
        return respond(
            &mut stream,
            "403 Forbidden",
            r#"{"error":"requests must use a loopback Host header"}"#,
        );
    }
    if path == "/v1/health" {
        return respond(
            &mut stream,
            "200 OK",
            &json!({"ok": true, "instanceId": state::state().instance_id}).to_string(),
        );
    }
    let supplied = headers
        .get("authorization")
        .and_then(|h| h.strip_prefix("Bearer "))
        .map(str::to_string)
        .or_else(|| {
            query
                .split('&')
                .find_map(|p| p.strip_prefix("token="))
                .map(str::to_string)
        })
        .unwrap_or_default();
    if !token().is_some_and(|t| same(&t, &supplied)) {
        return respond(
            &mut stream,
            "401 Unauthorized",
            r#"{"error":"a valid bearer token is required (the http-token file of this instance)"}"#,
        );
    }
    let client = headers
        .get("x-agentgo-client")
        .cloned()
        .unwrap_or_else(|| "http".into());
    match (verb, path) {
        ("POST", "/v1/call") => {
            let length = headers
                .get("content-length")
                .and_then(|l| l.parse::<usize>().ok())
                .unwrap_or(0)
                .min(8 * 1024 * 1024);
            let mut body = vec![0u8; length];
            if reader.read_exact(&mut body).is_err() {
                return respond(
                    &mut stream,
                    "400 Bad Request",
                    r#"{"error":"incomplete body"}"#,
                );
            }
            let Ok(value) = serde_json::from_slice::<Value>(&body) else {
                return respond(
                    &mut stream,
                    "400 Bad Request",
                    r#"{"error":"the body must be JSON {method, params}"}"#,
                );
            };
            let request = Request::new(
                value["id"].as_u64().unwrap_or(1),
                value["method"].as_str().unwrap_or(""),
                value.get("params").cloned().unwrap_or(json!({})),
            );
            // One HTTP client name is one session, so its tab leases persist.
            let session = format!(
                "http-{}",
                client
                    .chars()
                    .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
                    .take(32)
                    .collect::<String>()
            );
            let known = session.clone();
            let label = client.clone();
            exec::post(move || {
                if !sessions::status()["sessions"]
                    .as_array()
                    .is_some_and(|all| all.iter().any(|s| s["session"] == known.as_str()))
                {
                    let (sink, _) = mpsc::channel();
                    sessions::register(&known, &label, "http", sink);
                }
            });
            let line = call_on_ui(session, request);
            respond(&mut stream, "200 OK", line.trim_end());
        }
        ("GET", "/v1/ws") => {
            let Some(key) = headers.get("sec-websocket-key") else {
                return respond(
                    &mut stream,
                    "400 Bad Request",
                    r#"{"error":"a WebSocket upgrade is required"}"#,
                );
            };
            let accept = base64(&sha1(
                format!("{key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11").as_bytes(),
            ));
            if write!(stream, "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n").is_err() {
                return;
            }
            let _ = stream.set_read_timeout(None);
            let session = format!("ws-{number}");
            let (tx, rx) = mpsc::channel::<String>();
            let mut writer = stream.try_clone().expect("stream");
            std::thread::spawn(move || {
                while let Ok(line) = rx.recv() {
                    if writer.write_all(&ws_frame(line.trim_end())).is_err() {
                        break;
                    }
                }
            });
            let (id, out, label) = (session.clone(), tx.clone(), client.clone());
            exec::post(move || sessions::register(&id, &label, "websocket", out));
            state::wake_ui();
            while let Some((opcode, payload)) = ws_read(&mut reader) {
                match opcode {
                    0x1 => match serde_json::from_slice::<Request>(&payload) {
                        Ok(request) => {
                            let caller = Caller {
                                session: session.clone(),
                            };
                            exec::post(move || dispatch::handle_request(caller, request));
                            state::wake_ui();
                        }
                        Err(_) => {
                            let _ = tx.send(dispatch::encode(
                                0,
                                Err(Fail::new(
                                    ErrorCode::InvalidEnvelope,
                                    "a text frame carries one request {v, id, method, params}",
                                )),
                            ));
                        }
                    },
                    0x9 => {
                        let _ = stream.write_all(&[0x8A, 0]);
                    }
                    0x8 => break,
                    _ => {}
                }
            }
            let _ = stream.write_all(&[0x88, 0]);
            let id = session.clone();
            exec::post(move || sessions::unregister(&id));
            state::wake_ui();
        }
        _ => respond(
            &mut stream,
            "404 Not Found",
            r#"{"error":"POST /v1/call or GET /v1/ws"}"#,
        ),
    }
}

fn start(port: u16) -> Result<u16, Fail> {
    if PORT.load(Ordering::SeqCst) != 0 {
        return Ok(PORT.load(Ordering::SeqCst));
    }
    let listener = TcpListener::bind(("127.0.0.1", port))
        .map_err(|e| Fail::internal(format!("could not listen on 127.0.0.1:{port}: {e}")))?;
    let bound = listener
        .local_addr()
        .map_err(|e| Fail::internal(e.to_string()))?
        .port();
    let mut secret = [0u8; 32];
    getrandom::fill(&mut secret).map_err(|e| Fail::internal(e.to_string()))?;
    let value: String = secret.iter().map(|b| format!("{b:02x}")).collect();
    std::fs::write(token_file(), &value).map_err(|e| Fail::internal(e.to_string()))?;
    PORT.store(bound, Ordering::SeqCst);
    std::thread::Builder::new()
        .name("agentgo-http-accept".into())
        .spawn(move || {
            for (number, stream) in listener.incoming().enumerate() {
                if PORT.load(Ordering::SeqCst) != bound {
                    break; // turned off
                }
                let Ok(stream) = stream else { continue };
                if CONNECTIONS.load(Ordering::SeqCst) >= 32 {
                    continue;
                }
                CONNECTIONS.fetch_add(1, Ordering::SeqCst);
                let _ = std::thread::Builder::new()
                    .name("agentgo-http".into())
                    .spawn(move || {
                        serve(stream, number + 1);
                        CONNECTIONS.fetch_sub(1, Ordering::SeqCst);
                    });
            }
        })
        .map_err(|e| Fail::internal(e.to_string()))?;
    Ok(bound)
}

fn stop() {
    let port = PORT.swap(0, Ordering::SeqCst);
    let _ = std::fs::remove_file(token_file());
    if port != 0 {
        // Wake the accept loop so it sees it was turned off.
        let _ = TcpStream::connect(("127.0.0.1", port));
    }
}

fn status() -> Value {
    let port = PORT.load(Ordering::SeqCst);
    json!({
        "listening": port != 0,
        "port": port,
        "http": (port != 0).then(|| format!("http://127.0.0.1:{port}/v1/call")),
        "websocket": (port != 0).then(|| format!("ws://127.0.0.1:{port}/v1/ws")),
        "tokenFile": (port != 0).then(|| token_file().display().to_string()),
        "auth": "Authorization: Bearer <token from tokenFile> (or ?token= for WebSocket)",
    })
}

fn repo_sdk() -> std::path::PathBuf {
    let ui = state::state().options.ui_root.clone();
    let base = ui
        .parent()
        .map(std::path::Path::to_path_buf)
        .unwrap_or_default();
    base.join("sdk")
}

fn package(lang: &str) -> Value {
    let dir = repo_sdk().join(lang);
    let files: Vec<String> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    let (entry, usage) = match lang {
        "rust" => (
            "src/lib.rs",
            "agentgo::Client::from_token_file(port, path)?.call(\"observe.page\", json)",
        ),
        "typescript" => (
            "agentgo.mjs",
            "const ag = await AgentGo.connect({port, tokenFile}); await ag.call('observe.page')",
        ),
        "python" => (
            "agentgo.py",
            "ag = AgentGo(port, token_file); ag.call('observe.page')",
        ),
        "plugin" => (
            "template/skill.json",
            "skill.install_local {path: '<this folder>/template'}",
        ),
        _ => ("", ""),
    };
    json!({"language": lang, "path": dir.display().to_string(), "present": dir.join(entry).exists(), "entry": entry, "files": files, "usage": usage, "transport": status()})
}

pub fn handles(method: &str) -> bool {
    matches!(
        method,
        "sdk.http"
            | "sdk.websocket"
            | "sdk.rust"
            | "sdk.typescript"
            | "sdk.python"
            | "sdk.plugin"
            | "sdk.skill"
            | "sdk.test"
    )
}

pub async fn command(caller: &Caller, method: &str, params: &Value) -> Reply {
    match method {
        "sdk.http" | "sdk.websocket" => {
            if let Some(on) = params["on"].as_bool() {
                if !caller.is_shell() {
                    return Err(Fail::new(
                        ErrorCode::PolicyDenied,
                        "opening the local HTTP/WebSocket port is the person's decision",
                    ));
                }
                if on {
                    start(params["port"].as_u64().unwrap_or(0).min(65_535) as u16)?;
                } else {
                    stop();
                }
            }
            Ok(status())
        }
        "sdk.rust" => Ok(package("rust")),
        "sdk.typescript" => Ok(package("typescript")),
        "sdk.python" => Ok(package("python")),
        "sdk.plugin" => {
            let mut out = package("plugin");
            out["kinds"] = json!([
                {"kind": "skill", "how": "a folder with skill.json (steps with {{params}}), installed with skill.install_local"},
                {"kind": "user script", "how": "script.user_install (the person installs it), runs on matching pages in its own world"},
                {"kind": "extension", "how": "extension.install: the content scripts of a Chrome extension"},
            ]);
            Ok(out)
        }
        "sdk.skill" => {
            let skill = state::state()
                .options
                .ui_root
                .parent()
                .map(|p| p.join("skills").join("agentgo").join("SKILL.md"))
                .unwrap_or_default();
            let text = std::fs::read_to_string(&skill).unwrap_or_default();
            let version = text
                .lines()
                .find(|l| {
                    l.to_lowercase().contains("skill")
                        && l.chars().any(|c| c.is_ascii_digit())
                        && l.contains('.')
                })
                .unwrap_or("")
                .trim()
                .to_string();
            Ok(
                json!({"path": skill.display().to_string(), "present": !text.is_empty(), "version": version, "mcp": "claude mcp add --scope user agentgo -- agentgo mcp", "bytes": text.len()}),
            )
        }
        "sdk.test" => {
            // A real round trip through each transport that is up.
            let mut checks = vec![
                json!({"transport": "pipe", "ok": std::path::Path::new(&state::state().paths.instance_dir(&state::state().instance_id).join("token")).exists(), "detail": super::pipe::pipe_name(&state::state().instance_id)}),
            ];
            let port = PORT.load(Ordering::SeqCst);
            if port != 0 {
                let secret = token().unwrap_or_default();
                let reply = exec::blocking(20_000, move || -> Result<String, String> {
                    let mut stream = TcpStream::connect(("127.0.0.1", port)).map_err(|e| e.to_string())?;
                    let body = r#"{"method":"browser.version","params":{}}"#;
                    write!(stream, "POST /v1/call HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {secret}\r\nX-AgentGo-Client: sdk-test\r\nContent-Length: {}\r\n\r\n{body}", body.len()).map_err(|e| e.to_string())?;
                    let mut text = String::new();
                    stream.read_to_string(&mut text).map_err(|e| e.to_string())?;
                    Ok(text)
                })
                .await;
                let ok = matches!(&reply, Some(Ok(text)) if text.contains("\"ok\":true"));
                checks.push(
                    json!({"transport": "http", "ok": ok, "detail": format!("127.0.0.1:{port}")}),
                );
            } else {
                checks.push(json!({"transport": "http", "ok": null, "detail": "off (sdk.http {on: true} by the person)"}));
            }
            for lang in ["rust", "typescript", "python"] {
                let p = package(lang);
                checks.push(json!({"transport": format!("sdk/{lang}"), "ok": p["present"], "detail": p["path"]}));
            }
            let passed = checks.iter().all(|c| c["ok"] != false);
            Ok(json!({"passed": passed, "checks": checks}))
        }
        _ => Err(Fail::internal(format!("{method} is not an SDK command"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_loopback_hosts_and_origins_pass() {
        for ok in ["127.0.0.1:4000", "localhost", "[::1]:9", "LOCALHOST:1"] {
            assert!(is_loopback_host(ok), "{ok}");
        }
        for bad in [
            "evil.com",
            "127.0.0.1.evil.com",
            "localhost.evil.com:80",
            "[::1]x",
            "",
        ] {
            assert!(!is_loopback_host(bad), "{bad}");
        }
        assert!(is_loopback_origin("http://127.0.0.1:5000"));
        assert!(!is_loopback_origin("http://127.0.0.1.evil.com"));
        assert!(!is_loopback_origin("https://localhost"));
        assert!(!is_loopback_origin("null"));
    }

    #[test]
    fn websocket_accept_key() {
        // The example of RFC 6455, section 1.3.
        assert_eq!(
            base64(&sha1(
                b"dGhlIHNhbXBsZSBub25jZQ==258EAFA5-E914-47DA-95CA-C5AB0DC85B11"
            )),
            "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
        );
        assert_eq!(base64(&sha1(b"abc")), "qZk+NkcGgWq6PiVxeFDCbJzQ2J0=");
    }
}
