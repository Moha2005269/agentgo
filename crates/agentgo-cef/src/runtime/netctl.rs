//! Network control for agents: requests blocked, answered with a prepared
//! response, or failed on purpose, by address pattern; bodies and headers of
//! what the page loaded; WebSocket and server-sent-event logs; HAR capture
//! and replay. Rules belong to one tab and end with `network.clear_interceptors`.

use std::cell::RefCell;
use std::collections::HashMap;

use serde_json::{json, Value};

use super::sessions::{Caller, Fail, Reply};
use super::{cdp, page};

#[derive(Clone)]
struct Rule {
    pattern: String,
    action: String,
    status: u16,
    body: String,
    content_type: String,
    reason: String,
}

thread_local! {
    static RULES: RefCell<HashMap<String, Vec<Rule>>> = RefCell::new(HashMap::new());
}

fn glob(pattern: &str, text: &str) -> bool {
    match pattern.split_once('*') {
        None => pattern == text,
        Some((head, tail)) => {
            text.starts_with(head)
                && (head.len()..=text.len())
                    .any(|at| text.is_char_boundary(at) && glob(tail, &text[at..]))
        }
    }
}

fn encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n =
            chunk.iter().fold(0u32, |acc, b| acc << 8 | u32::from(*b)) << (8 * (3 - chunk.len()));
        for i in 0..4 {
            out.push(if i <= chunk.len() {
                TABLE[(n >> (18 - 6 * i) & 63) as usize] as char
            } else {
                '='
            });
        }
    }
    out
}

/// Answers a request the page paused at (DevTools `Fetch.requestPaused`).
pub async fn on_paused(tab: String, params: Value) {
    let id = params["requestId"].clone();
    let url = params["request"]["url"].as_str().unwrap_or("").to_string();
    let rule = RULES.with(|all| {
        all.borrow()
            .get(&tab)
            .and_then(|rules| rules.iter().find(|rule| glob(&rule.pattern, &url)).cloned())
    });
    let _ = match rule.as_ref().map(|rule| rule.action.as_str()) {
        Some("block") | Some("fail") => {
            let reason = rule
                .as_ref()
                .map(|r| r.reason.clone())
                .filter(|r| !r.is_empty())
                .unwrap_or_else(|| "BlockedByClient".into());
            cdp::call(
                &tab,
                "Fetch.failRequest",
                json!({"requestId": id, "errorReason": reason}),
            )
            .await
        }
        Some("fulfill") => {
            let rule = rule.unwrap_or_else(|| unreachable!());
            cdp::call(&tab, "Fetch.fulfillRequest", json!({
                "requestId": id, "responseCode": rule.status,
                "responseHeaders": [{"name": "Content-Type", "value": rule.content_type}, {"name": "Access-Control-Allow-Origin", "value": "*"}],
                "body": encode(rule.body.as_bytes()),
            })).await
        }
        _ => cdp::call(&tab, "Fetch.continueRequest", json!({"requestId": id})).await,
    };
}

async fn enable_interception(tab: &str) -> Result<(), Fail> {
    cdp::call(
        tab,
        "Fetch.enable",
        json!({"patterns": [{"urlPattern": "*"}]}),
    )
    .await
    .map(|_| ())
    .map_err(Fail::internal)
}

fn find_request(tab: &str, id: &str) -> Option<Value> {
    cdp::events(tab, &["request"], 400, false)
        .into_iter()
        .rev()
        .find(|event| event["id"] == id)
}

/// Every `network.*` command of this module.
pub async fn command(caller: &Caller, method: &str, params: &Value) -> Reply {
    let tab = page::tab_of(caller, params)?;
    let _ = cdp::call(&tab, "Network.enable", json!({})).await;
    let text = |key: &str| params[key].as_str().map(str::to_string);
    match method {
        "network.intercept"
        | "network.fulfill"
        | "network.modify_response"
        | "network.emulate_errors"
        | "network.continue" => {
            let pattern = text("pattern").ok_or_else(|| {
                Fail::params("params.pattern: an address pattern such as *://*/api/*")
            })?;
            let action = match method {
                "network.fulfill" | "network.modify_response" => "fulfill".to_string(),
                "network.emulate_errors" => "fail".to_string(),
                "network.continue" => "continue".to_string(),
                _ => text("action").unwrap_or_else(|| "block".into()),
            };
            if !matches!(action.as_str(), "block" | "fulfill" | "fail" | "continue") {
                return Err(Fail::params("action is block, fulfill, fail or continue"));
            }
            let reasons = [
                "Failed",
                "Aborted",
                "TimedOut",
                "AccessDenied",
                "ConnectionClosed",
                "ConnectionReset",
                "ConnectionRefused",
                "ConnectionAborted",
                "ConnectionFailed",
                "NameNotResolved",
                "InternetDisconnected",
                "AddressUnreachable",
                "BlockedByClient",
                "BlockedByResponse",
            ];
            let reason = text("reason").unwrap_or_else(|| {
                if action == "fail" {
                    "ConnectionFailed".into()
                } else {
                    String::new()
                }
            });
            if !reason.is_empty() && !reasons.contains(&reason.as_str()) {
                return Err(Fail::params(format!(
                    "reason is one of {}",
                    reasons.join(", ")
                )));
            }
            let rule = Rule {
                pattern: pattern.clone(),
                action: action.clone(),
                status: params["status"].as_u64().unwrap_or(200).clamp(100, 599) as u16,
                body: params
                    .get("body")
                    .map(|b| {
                        b.as_str()
                            .map(str::to_string)
                            .unwrap_or_else(|| b.to_string())
                    })
                    .unwrap_or_default(),
                content_type: text("contentType").unwrap_or_else(|| {
                    if params["body"].is_object() || params["body"].is_array() {
                        "application/json".into()
                    } else {
                        "text/plain; charset=utf-8".into()
                    }
                }),
                reason,
            };
            // Newer rules win over older ones for the same addresses.
            RULES.with(|all| {
                all.borrow_mut()
                    .entry(tab.clone())
                    .or_default()
                    .insert(0, rule)
            });
            enable_interception(&tab).await?;
            let count = RULES.with(|all| all.borrow().get(&tab).map_or(0, Vec::len));
            Ok(json!({"tabId": tab, "pattern": pattern, "action": action, "rules": count}))
        }
        "network.clear_interceptors" => {
            let removed = RULES
                .with(|all| all.borrow_mut().remove(&tab))
                .map_or(0, |rules| rules.len());
            let _ = cdp::call(&tab, "Fetch.disable", json!({})).await;
            Ok(json!({"tabId": tab, "removed": removed}))
        }
        "network.request_body" => {
            let id = text("requestId")
                .ok_or_else(|| Fail::params("params.requestId (from network.observe_requests)"))?;
            let body = cdp::call(&tab, "Network.getRequestPostData", json!({"requestId": id}))
                .await
                .map_err(|_| Fail::params("that request has no body, or it is no longer kept"))?;
            Ok(json!({"requestId": id, "body": body["postData"]}))
        }
        "network.response_body" | "network.download_response" => {
            let id = text("requestId")
                .ok_or_else(|| Fail::params("params.requestId (from network.observe_requests)"))?;
            let body = cdp::call(&tab, "Network.getResponseBody", json!({"requestId": id}))
                .await
                .map_err(|_| {
                    Fail::params(
                        "that response body is not available (not loaded yet, or no longer kept)",
                    )
                })?;
            let encoded = body["base64Encoded"] == true;
            let raw = body["body"].as_str().unwrap_or("");
            if method == "network.download_response" {
                let bytes = if encoded {
                    page::decode_base64(raw)
                } else {
                    raw.as_bytes().to_vec()
                };
                return page::save_artifact(
                    caller,
                    "response",
                    if encoded { "bin" } else { "txt" },
                    &bytes,
                );
            }
            Ok(
                json!({"requestId": id, "base64": encoded, "body": raw.chars().take(200_000).collect::<String>(), "truncated": raw.len() > 200_000}),
            )
        }
        "network.headers" => {
            let id = text("requestId")
                .ok_or_else(|| Fail::params("params.requestId (from network.observe_requests)"))?;
            let request = find_request(&tab, &id)
                .ok_or_else(|| Fail::params("no such request in the recent log"))?;
            let response = cdp::events(&tab, &["response"], 400, false)
                .into_iter()
                .rev()
                .find(|event| event["id"] == id.as_str());
            Ok(
                json!({"requestId": id, "url": request["url"], "request": request["headers"], "response": response.map(|r| r["headers"].clone())}),
            )
        }
        "network.websocket_list" => Ok(json!({"sockets": cdp::events(&tab, &["ws"], 100, false)})),
        "network.websocket_messages" => Ok(
            json!({"messages": cdp::events(&tab, &["ws-frame"], params["max"].as_u64().unwrap_or(100) as usize, false)}),
        ),
        "network.sse_events" => Ok(
            json!({"events": cdp::events(&tab, &["sse"], params["max"].as_u64().unwrap_or(100) as usize, false)}),
        ),
        "network.har_stop" => {
            let requests = cdp::events(&tab, &["request"], 400, false);
            let responses = cdp::events(&tab, &["response"], 400, false);
            let entries: Vec<Value> = requests.iter().map(|request| {
                let response = responses.iter().find(|r| r["id"] == request["id"]);
                json!({
                    "request": {"method": request["method"], "url": request["url"], "headers": request["headers"].as_object().map(|h| h.iter().map(|(n, v)| json!({"name": n, "value": v})).collect::<Vec<_>>()).unwrap_or_default()},
                    "response": {"status": response.map(|r| r["status"].clone()), "content": {"mimeType": response.map(|r| r["mime"].clone())}},
                })
            }).collect();
            let har = json!({"log": {"version": "1.2", "creator": {"name": "AgentGo", "version": env!("CARGO_PKG_VERSION")}, "entries": entries}});
            let mut saved = page::save_artifact(
                caller,
                "network",
                "har",
                serde_json::to_string_pretty(&har)
                    .unwrap_or_default()
                    .as_bytes(),
            )?;
            saved["entries"] = json!(entries.len());
            Ok(saved)
        }
        _ => {
            // network.replay: the same request again, from the page itself.
            let id = text("requestId")
                .ok_or_else(|| Fail::params("params.requestId (from network.observe_requests)"))?;
            let request = find_request(&tab, &id)
                .ok_or_else(|| Fail::params("no such request in the recent log"))?;
            let expression = format!(
                "fetch({}, {{method: {}, credentials: 'include'}}).then(r => r.status, e => 'failed: ' + e.message)",
                request["url"], request["method"]
            );
            let result = cdp::call(
                &tab,
                "Runtime.evaluate",
                json!({"expression": expression, "awaitPromise": true, "returnByValue": true}),
            )
            .await
            .map_err(Fail::internal)?;
            Ok(json!({"requestId": id, "url": request["url"], "status": result["result"]["value"]}))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns_and_bodies() {
        assert!(glob("*://*/api/*", "https://shop.example/api/items?x=1"));
        assert!(!glob("*://*/api/*", "https://shop.example/home"));
        assert!(glob("*.png", "https://a.example/x.png"));
        assert_eq!(encode(b"hi!"), "aGkh");
        assert_eq!(encode(b"hi"), "aGk=");
    }
}
