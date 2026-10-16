//! Newline-delimited JSON framing used by the stdio transport and the
//! named-pipe transport (Milestone 3).
//!
//! Rules:
//! * one JSON document per `\n`-terminated line;
//! * no embedded raw newlines (serde_json never emits them);
//! * decode is incremental and bounds-checked — a frame longer than
//!   `FrameConfig::max_frame_bytes` is an error, never an unbounded buffer.

use crate::ProtocolError;

/// Framing limits. Defaults are deliberately conservative; transports may
/// tighten them further.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameConfig {
    /// Maximum encoded frame (line) length in bytes.
    pub max_frame_bytes: usize,
}

impl Default for FrameConfig {
    fn default() -> Self {
        // 4 MiB comfortably covers any observation snapshot or workflow
        // result we intentionally return; larger payloads are meant to be
        // artifacts with handles, not inline frames.
        Self {
            max_frame_bytes: 4 * 1024 * 1024,
        }
    }
}

/// Encodes envelopes (any serializable message) into single-line frames
/// terminated by `\n`.
#[derive(Debug, Default)]
pub struct FrameEncoder;

impl FrameEncoder {
    /// Serializes `msg` and appends the newline terminator.
    pub fn encode<T: serde::Serialize>(&self, msg: &T) -> Result<String, ProtocolError> {
        let mut line = serde_json::to_string(msg).map_err(|e| ProtocolError::InvalidFrame {
            detail: e.to_string(),
        })?;
        if line.contains('\n') || line.contains('\r') {
            return Err(ProtocolError::InvalidFrame {
                detail: "serialized message contains line breaks".into(),
            });
        }
        line.push('\n');
        Ok(line)
    }
}

/// Incremental decoder that accepts bytes as they arrive and yields complete
/// frames. Enforces the configured frame-length bound.
#[derive(Debug)]
pub struct FrameDecoder {
    config: FrameConfig,
    buffer: Vec<u8>,
}

impl FrameDecoder {
    /// Creates a decoder with the given limits.
    pub fn new(config: FrameConfig) -> Self {
        Self {
            config,
            buffer: Vec::new(),
        }
    }

    /// Feeds raw bytes into the decoder.
    pub fn push(&mut self, bytes: &[u8]) -> Result<(), ProtocolError> {
        if self.buffer.len() + bytes.len() > self.config.max_frame_bytes {
            self.buffer.clear();
            return Err(ProtocolError::FrameTooLong {
                max: self.config.max_frame_bytes,
            });
        }
        self.buffer.extend_from_slice(bytes);
        Ok(())
    }

    /// Pops the next complete frame (without the trailing newline), if any.
    /// Incomplete data stays buffered for the next push.
    pub fn pop_frame(&mut self) -> Option<Result<String, ProtocolError>> {
        let idx = self.buffer.iter().position(|&b| b == b'\n')?;
        let mut line: Vec<u8> = self.buffer.drain(..=idx).collect();
        line.pop(); // trailing \n
        if line.last() == Some(&b'\r') {
            line.pop();
        }
        match String::from_utf8(line) {
            Ok(s) => Some(Ok(s)),
            Err(e) => Some(Err(ProtocolError::InvalidFrame {
                detail: format!("invalid UTF-8: {e}"),
            })),
        }
    }

    /// Decodes the next complete frame as JSON into `T`.
    pub fn pop_message<T: serde::de::DeserializeOwned>(
        &mut self,
    ) -> Option<Result<T, ProtocolError>> {
        let frame = self.pop_frame()?;
        Some(frame.and_then(|frame| {
            serde_json::from_str(&frame).map_err(|e| ProtocolError::InvalidFrame {
                detail: e.to_string(),
            })
        }))
    }

    /// Bytes currently buffered awaiting a newline.
    pub fn buffered(&self) -> usize {
        self.buffer.len()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::envelope::Request;
    use serde_json::json;

    fn request(id: u64, method: &str) -> Request {
        Request::new(id, method, json!({}))
    }

    #[test]
    fn encode_decode_roundtrip() {
        let enc = FrameEncoder;
        let line = enc.encode(&request(1, "browser.version")).unwrap();
        assert!(line.ends_with('\n'));
        let mut dec = FrameDecoder::new(FrameConfig::default());
        dec.push(line.as_bytes()).unwrap();
        let msg: Request = dec.pop_message().unwrap().unwrap();
        assert_eq!(msg.id, 1);
        assert_eq!(msg.method, "browser.version");
        assert_eq!(dec.buffered(), 0);
    }

    #[test]
    fn partial_feed_accumulates() {
        let enc = FrameEncoder;
        let line = enc.encode(&request(2, "tab.list")).unwrap();
        let bytes = line.as_bytes();
        let mut dec = FrameDecoder::new(FrameConfig::default());
        // Feed everything except the final newline in 3-byte chunks; the
        // frame must never be complete early.
        let mut i = 0;
        while i < bytes.len() - 1 {
            let end = (i + 3).min(bytes.len() - 1);
            dec.push(&bytes[i..end]).unwrap();
            assert!(
                dec.pop_frame().is_none(),
                "frame must not be complete early"
            );
            i = end;
        }
        // The terminating newline completes the frame.
        dec.push(&bytes[bytes.len() - 1..]).unwrap();
        let msg: Request = dec.pop_message().unwrap().unwrap();
        assert_eq!(msg.id, 2);
    }

    #[test]
    fn multiple_frames_queue_in_order() {
        let enc = FrameEncoder;
        let l1 = enc.encode(&request(1, "a.b")).unwrap();
        let l2 = enc.encode(&request(2, "c.d")).unwrap();
        let mut dec = FrameDecoder::new(FrameConfig::default());
        dec.push(l1.as_bytes()).unwrap();
        dec.push(l2.as_bytes()).unwrap();
        let m1: Request = dec.pop_message().unwrap().unwrap();
        let m2: Request = dec.pop_message().unwrap().unwrap();
        assert_eq!((m1.id, m2.id), (1, 2));
        assert!(dec.pop_frame().is_none());
    }

    #[test]
    fn oversize_frame_rejected() {
        let mut dec = FrameDecoder::new(FrameConfig {
            max_frame_bytes: 64,
        });
        let big = vec![b'x'; 65];
        match dec.push(&big) {
            Err(ProtocolError::FrameTooLong { max: 64 }) => {}
            other => panic!("expected FrameTooLong, got {other:?}"),
        }
    }

    #[test]
    fn invalid_json_is_invalid_frame() {
        let mut dec = FrameDecoder::new(FrameConfig::default());
        dec.push(b"this is not json\n").unwrap();
        match dec.pop_message::<Request>() {
            Some(Err(ProtocolError::InvalidFrame { .. })) => {}
            other => panic!("expected InvalidFrame, got {other:?}"),
        }
    }

    #[test]
    fn crlf_tolerated() {
        let mut dec = FrameDecoder::new(FrameConfig::default());
        dec.push(b"{\"v\":1,\"id\":5,\"method\":\"x.y\",\"params\":null}\r\n")
            .unwrap();
        let msg: Request = dec.pop_message().unwrap().unwrap();
        assert_eq!(msg.id, 5);
    }

    /// Hostile input: random bytes, broken UTF-8, huge lines, deeply nested
    /// JSON and valid frames cut at random places. The decoder must never
    /// panic, never hold more than its limit, and still deliver every valid
    /// frame that arrives whole between the garbage.
    #[test]
    fn hostile_streams_never_panic_or_grow_without_bound() {
        // xorshift64*: deterministic, so a failure can be replayed.
        let mut seed = 0x9E37_79B9_7F4A_7C15_u64;
        let mut next = move || {
            seed ^= seed >> 12;
            seed ^= seed << 25;
            seed ^= seed >> 27;
            seed.wrapping_mul(0x2545_F491_4F6C_DD1D)
        };
        let config = FrameConfig {
            max_frame_bytes: 4096,
        };
        let mut decoder = FrameDecoder::new(config);
        let valid = FrameEncoder.encode(&request(7, "tabs.list")).unwrap();
        let mut sent = 0_u32;
        let mut received = 0_u32;
        for round in 0..20_000_u32 {
            let chunk: Vec<u8> = match next() % 7 {
                0 => (0..next() % 64).map(|_| next() as u8).collect(),
                1 => vec![b'['; (next() % 3000) as usize],
                2 => b"{\"v\":1,\"id\":".to_vec(),
                3 => vec![0xFF, 0xFE, b'\n'],
                4 => vec![b'x'; (next() % 6000) as usize],
                5 => b"\r\n\n\n".to_vec(),
                _ => {
                    // A whole valid frame on a clean line.
                    let _ = decoder.push(b"\n");
                    while decoder.pop_message::<Request>().is_some() {}
                    sent += 1;
                    valid.as_bytes().to_vec()
                }
            };
            let clean_frame = chunk == valid.as_bytes();
            let pushed = decoder.push(&chunk).is_ok();
            assert!(
                decoder.buffered() <= config.max_frame_bytes,
                "round {round}"
            );
            while let Some(message) = decoder.pop_message::<Request>() {
                if let Ok(request) = message {
                    assert_eq!(request.method, "tabs.list");
                    received += 1;
                }
            }
            if clean_frame {
                assert!(pushed, "a valid frame was refused in round {round}");
            }
        }
        assert!(sent > 1000);
        assert_eq!(received, sent, "valid frames were lost");
    }
}
