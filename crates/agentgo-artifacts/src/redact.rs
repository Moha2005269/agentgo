//! Secret redaction for logs, traces, artifacts, and protocol payloads.
//!
//! Redaction runs before persistence or emission — never after. The
//! implementation is deliberately conservative: it targets the secret
//! categories named in the plan (authorization headers, cookies, tokens,
//! passwords, API keys, credentials in forms) and replaces values with a
//! stable, non-reversible placeholder that keeps field shape intact.

use serde_json::{Map, Value};

/// The placeholder substituted for secret values.
pub const REDACTED: &str = "[REDACTED]";

const HEADER_NAMES: &[&str] = &[
    "authorization",
    "proxy-authorization",
    "cookie",
    "set-cookie",
    "x-api-key",
    "x-auth-token",
    "x-session-id",
];

const FIELD_NAMES: &[&str] = &[
    "password",
    "passwd",
    "pass",
    "secret",
    "token",
    "accesstoken",
    "access_token",
    "idtoken",
    "id_token",
    "refreshtoken",
    "refresh_token",
    "apikey",
    "api_key",
    "clientsecret",
    "client_secret",
    "privatekey",
    "private_key",
    "credential",
    "credentials",
    "sessionid",
    "session_id",
    "cookie",
    "cookies",
    "authorization",
];

/// Redacts header maps (`{"Authorization": "Bearer xyz", ...}`).
/// Header-name matching is case-insensitive.
pub fn redact_headers(headers: &Map<String, Value>) -> Map<String, Value> {
    let mut out = Map::new();
    for (k, v) in headers {
        if HEADER_NAMES.contains(&k.to_ascii_lowercase().as_str()) {
            out.insert(k.clone(), Value::String(REDACTED.into()));
        } else {
            out.insert(k.clone(), v.clone());
        }
    }
    out
}

/// True when a flattened key name is a known secret field.
fn is_secret_key(key: &str) -> bool {
    let flat = key.replace(['-', '_', ' '], "").to_ascii_lowercase();
    FIELD_NAMES.contains(&flat.as_str())
}

/// Recursively redacts JSON objects whose keys name secret fields; scalar
/// secret values become `[REDACTED]` while object shape is preserved.
pub fn redact_json(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut out = Map::new();
            for (k, v) in map {
                if is_secret_key(k)
                    && matches!(v, Value::String(_) | Value::Number(_) | Value::Bool(_))
                {
                    out.insert(k.clone(), Value::String(REDACTED.into()));
                } else {
                    out.insert(k.clone(), redact_json(v));
                }
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(redact_json).collect()),
        other => other.clone(),
    }
}

/// Redacts inline `key=value` secret assignments in free text.
/// Non-matching text passes through unchanged.
pub fn redact_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(idx) = rest.find(['=', ':']) {
        let (head, tail) = rest.split_at(idx);
        // Word immediately before the separator.
        let word: String = head
            .chars()
            .rev()
            .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        let sep_len = 1;
        out.push_str(head);
        out.push_str(&tail[..sep_len]);
        if is_secret_key(&word) {
            out.push_str(REDACTED);
            let after = &tail[sep_len..];
            let value_len = after
                .find(|c: char| c.is_whitespace() || c == '&' || c == '"' || c == ',')
                .unwrap_or(after.len());
            rest = &after[value_len..];
        } else {
            rest = &tail[sep_len..];
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn headers_redacted_case_insensitively() {
        let mut h = Map::new();
        h.insert("Authorization".into(), json!("Bearer abc.def.ghi"));
        h.insert("set-COOKIE".into(), json!("sid=monster; Secure"));
        h.insert("Content-Type".into(), json!("application/json"));
        let r = redact_headers(&h);
        assert_eq!(r["Authorization"], json!(REDACTED));
        assert_eq!(r["set-COOKIE"], json!(REDACTED));
        assert_eq!(r["Content-Type"], json!("application/json"));
    }

    #[test]
    fn nested_json_fields_redacted_with_shape_preserved() {
        let v = json!({
            "form": { "username": "ada", "password": "hunter2", "nested": { "api_key": "sk-123" } },
            "flags": { "remember": true },
            "list": [ { "token": "t1" }, { "ok": "fine" } ]
        });
        let r = redact_json(&v);
        assert_eq!(r["form"]["username"], json!("ada"));
        assert_eq!(r["form"]["password"], json!(REDACTED));
        assert_eq!(r["form"]["nested"]["api_key"], json!(REDACTED));
        assert_eq!(r["flags"]["remember"], json!(true));
        assert_eq!(r["list"][0]["token"], json!(REDACTED));
        assert_eq!(r["list"][1]["ok"], json!("fine"));
    }

    #[test]
    fn non_secret_keys_untouched() {
        let v = json!({"tokener": "not-a-secret", "tokenizing": "also fine"});
        let r = redact_json(&v);
        assert_eq!(r["tokener"], json!("not-a-secret"));
        assert_eq!(r["tokenizing"], json!("also fine"));
    }

    #[test]
    fn inline_assignments_redacted_in_text() {
        let text = "POST body: username=ada&password=hunter2&token=abc123&submit=go";
        let r = redact_text(text);
        assert!(r.contains("username=ada"));
        assert!(r.contains(&format!("password={REDACTED}")));
        assert!(r.contains(&format!("token={REDACTED}")));
        assert!(r.contains("submit=go"));
        assert!(!r.contains("hunter2"));
        assert!(!r.contains("abc123"));
    }

    #[test]
    fn plain_text_without_secrets_unchanged() {
        let text = "navigate to https://www.google.com/search?q=agentgo";
        assert_eq!(redact_text(text), text);
    }
}
