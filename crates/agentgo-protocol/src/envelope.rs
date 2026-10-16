//! Typed message envelopes shared by all transports and the shell bridge.
//!
//! Wire shape (newline-delimited JSON on stdio / named pipe):
//!
//! ```json
//! {"v":1,"id":7,"method":"tab.navigate","params":{"tabId":"t-1","url":"https://www.google.com/"}}
//! {"v":1,"id":7,"ok":true,"result":{...}}
//! {"v":1,"id":null,"event":"tab.statusChanged","params":{...}}
//! ```
//!
//! `v` is the envelope **shape** version (not the negotiated protocol
//! version); it exists so framing can be upgraded independently.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Envelope shape version. Bump only with a framing-breaking change and a
/// matching entry in `FrameDecoder`.
pub const ENVELOPE_VERSION: u32 = 1;

/// A client -> AgentGo request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    /// Envelope shape version.
    pub v: u32,
    /// Correlation id chosen by the caller; echoed in the [`Response`].
    pub id: u64,
    /// Dotted command name, e.g. `tab.navigate`. Must exist in the
    /// capability manifest.
    pub method: String,
    /// Typed parameters for `method`.
    #[serde(default = "Value::default")]
    pub params: Value,
}

/// AgentGo -> client reply for a [`Request`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub v: u32,
    /// Correlation id of the originating request.
    pub id: u64,
    /// `true` when the command executed; when `false`, `error` is present.
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorBody>,
}

/// Typed error body carried on failed responses.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorBody {
    /// Stable machine-readable code (see [`crate::ErrorCode`]).
    pub code: String,
    /// Human-readable, non-secret explanation.
    pub message: String,
    /// Optional structured detail (never secrets; redacted upstream).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// AgentGo -> client (or shell) event notification.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub v: u32,
    /// Event name, e.g. `tab.statusChanged`.
    pub event: String,
    #[serde(default = "Value::default")]
    pub params: Value,
}

impl Request {
    /// Builds a request with the current envelope version.
    pub fn new(id: u64, method: impl Into<String>, params: Value) -> Self {
        Self {
            v: ENVELOPE_VERSION,
            id,
            method: method.into(),
            params,
        }
    }
}

impl Response {
    /// Successful response with a result value.
    pub fn ok(id: u64, result: Value) -> Self {
        Self {
            v: ENVELOPE_VERSION,
            id,
            ok: true,
            result: Some(result),
            error: None,
        }
    }

    /// Failed response with a typed error body.
    pub fn err(id: u64, code: crate::ErrorCode, message: impl Into<String>) -> Self {
        Self {
            v: ENVELOPE_VERSION,
            id,
            ok: false,
            result: None,
            error: Some(ErrorBody {
                code: code.as_str().to_owned(),
                message: message.into(),
                data: None,
            }),
        }
    }
}

impl Event {
    /// Builds an event with the current envelope version.
    pub fn new(event: impl Into<String>, params: Value) -> Self {
        Self {
            v: ENVELOPE_VERSION,
            event: event.into(),
            params,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_roundtrip() {
        let req = Request::new(
            7,
            "tab.navigate",
            serde_json::json!({"tabId": "t-1", "url": "https://www.google.com/"}),
        );
        let text = serde_json::to_string(&req).unwrap();
        assert!(
            !text.contains('\n'),
            "encoded envelope must stay on one line"
        );
        let back: Request = serde_json::from_str(&text).unwrap();
        assert_eq!(req, back);
    }

    #[test]
    fn ok_response_roundtrip_and_omits_error() {
        let res = Response::ok(9, serde_json::json!({"tabId": "t-1"}));
        let text = serde_json::to_string(&res).unwrap();
        assert!(!text.contains("\"error\""));
        let back: Response = serde_json::from_str(&text).unwrap();
        assert_eq!(res, back);
    }

    #[test]
    fn err_response_roundtrip() {
        let res = Response::err(
            11,
            crate::ErrorCode::PolicyDenied,
            "money actions require user approval",
        );
        assert!(!res.ok);
        let text = serde_json::to_string(&res).unwrap();
        assert!(text.contains("\"code\":\"policy.denied\""));
        let back: Response = serde_json::from_str(&text).unwrap();
        assert_eq!(res, back);
    }

    #[test]
    fn event_roundtrip() {
        let ev = Event::new(
            "tab.statusChanged",
            serde_json::json!({"tabId": "t-1", "loading": false}),
        );
        let back: Event = serde_json::from_str(&serde_json::to_string(&ev).unwrap()).unwrap();
        assert_eq!(ev, back);
    }

    #[test]
    fn params_default_to_null_when_missing() {
        let req: Request =
            serde_json::from_str(r#"{"v":1,"id":3,"method":"browser.capabilities"}"#).unwrap();
        assert_eq!(req.params, Value::Null);
    }
}
