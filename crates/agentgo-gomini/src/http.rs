//! A small HTTP/1.1 client for the worker on 127.0.0.1. Nothing else is
//! ever contacted with it, so it needs no TLS, proxies or redirects.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

/// Sends one request and returns the status code and body.
pub fn request(
    port: u16,
    method: &str,
    path: &str,
    body: Option<&str>,
    timeout: Duration,
) -> Result<(u16, String), String> {
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = TcpStream::connect_timeout(&address, timeout)
        .map_err(|e| format!("GoMini worker unreachable: {e}"))?;
    stream.set_read_timeout(Some(timeout)).ok();
    stream.set_write_timeout(Some(timeout)).ok();
    let body = body.unwrap_or("");
    let head = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream
        .write_all(head.as_bytes())
        .and_then(|()| stream.write_all(body.as_bytes()))
        .map_err(|e| format!("GoMini worker request failed: {e}"))?;
    let mut raw = Vec::new();
    stream
        .read_to_end(&mut raw)
        .map_err(|e| format!("GoMini worker answer failed: {e}"))?;
    parse(&raw)
}

fn parse(raw: &[u8]) -> Result<(u16, String), String> {
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or("GoMini worker sent no HTTP header")?;
    let head = String::from_utf8_lossy(&raw[..split]);
    let mut payload = raw[split + 4..].to_vec();
    let status = head
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .ok_or("GoMini worker sent no status")?;
    let chunked = head.lines().any(|line| {
        let line = line.to_ascii_lowercase();
        line.starts_with("transfer-encoding:") && line.contains("chunked")
    });
    if chunked {
        payload = dechunk(&payload)?;
    }
    Ok((status, String::from_utf8_lossy(&payload).into_owned()))
}

fn dechunk(mut data: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    loop {
        let end = data
            .windows(2)
            .position(|w| w == b"\r\n")
            .ok_or("bad chunked body")?;
        let size_text = String::from_utf8_lossy(&data[..end]);
        let size = usize::from_str_radix(size_text.split(';').next().unwrap_or("").trim(), 16)
            .map_err(|_| "bad chunk size")?;
        data = &data[end + 2..];
        if size == 0 {
            return Ok(out);
        }
        if data.len() < size {
            return Err("truncated chunked body".into());
        }
        out.extend_from_slice(&data[..size]);
        data = data.get(size + 2..).unwrap_or(&[]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_plain_and_chunked_answers() {
        let plain = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}";
        assert_eq!(parse(plain).unwrap(), (200, "{}".to_string()));
        let chunked =
            b"HTTP/1.1 400 Bad Request\r\nTransfer-Encoding: chunked\r\n\r\n3\r\n{\"a\r\n3\r\n\":1\r\n1\r\n}\r\n0\r\n\r\n";
        assert_eq!(parse(chunked).unwrap(), (400, "{\"a\":1}".to_string()));
        assert!(parse(b"garbage").is_err());
    }
}
