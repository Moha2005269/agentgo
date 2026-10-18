//! JSON text exactly as Python's `json.dumps(value, ensure_ascii=False)`
//! writes it: `", "` and `": "` separators, keys in the order given.
//!
//! GoMini was trained on states serialised that way; the same characters
//! must reach it at inference or the tokens differ from training.

use serde_json::Value;

/// A JSON value whose object keys keep the order they were built in.
#[derive(Debug, Clone, PartialEq)]
pub enum Py {
    /// An object, keys in insertion order.
    Obj(Vec<(String, Py)>),
    /// Any other value. Objects inside it come from AgentGo's own JSON,
    /// whose keys are already sorted, as Python received them.
    Val(Value),
}

/// Serialises like Python's `json.dumps(..., ensure_ascii=False)`.
pub fn dumps(value: &Py) -> String {
    let mut out = String::new();
    write_py(value, &mut out);
    out
}

/// A JSON string literal (same escaping as Python with `ensure_ascii=False`).
pub fn string(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_else(|_| "\"\"".into())
}

fn write_py(value: &Py, out: &mut String) {
    match value {
        Py::Obj(pairs) => {
            out.push('{');
            for (i, (key, item)) in pairs.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                out.push_str(&string(key));
                out.push_str(": ");
                write_py(item, out);
            }
            out.push('}');
        }
        Py::Val(value) => write_value(value, out),
    }
}

fn write_value(value: &Value, out: &mut String) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => match n.as_f64() {
            // Python writes 1.0 where serde_json may write 1.0 too; integers stay integers.
            Some(f) if n.is_f64() && f.fract() == 0.0 && f.abs() < 1e16 => {
                out.push_str(&format!("{f:.1}"))
            }
            _ => out.push_str(&n.to_string()),
        },
        Value::String(s) => out.push_str(&string(s)),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write_value(item, out);
            }
            out.push(']');
        }
        Value::Object(map) => {
            out.push('{');
            for (i, (key, item)) in map.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                out.push_str(&string(key));
                out.push_str(": ");
                write_value(item, out);
            }
            out.push('}');
        }
    }
}

/// A protocol result as the MCP server's `render` shows it to a model
/// (`title — url`, then the page or text body). GoMini's training data was
/// read through MCP, so its `text` field has this shape. Keep in step with
/// `agentgo-mcp`'s `render`.
pub fn mcp_render(result: &Value) -> String {
    let Some(map) = result.as_object() else {
        return result.to_string();
    };
    let mut out = Vec::new();
    let mut rest = map.clone();
    let view = ["page", "text", "changes"]
        .iter()
        .any(|key| map.contains_key(*key));
    if !view {
        if let Some(tab) = rest.remove("tabId").filter(|value| !value.is_null()) {
            out.push(format!(
                "tabId: {}",
                tab.as_str().map_or_else(|| tab.to_string(), str::to_string)
            ));
        }
    }
    let mut take = |key: &str| rest.remove(key).filter(|value| !value.is_null());
    if let (Some(url), title) = (take("url"), take("title")) {
        let title = title
            .and_then(|t| t.as_str().map(str::to_string))
            .unwrap_or_default();
        out.push(
            format!("{title} — {}", url.as_str().unwrap_or(""))
                .trim_start_matches(" — ")
                .to_string(),
        );
    }
    for key in ["page", "text", "changes", "filled"] {
        if let Some(Value::String(body)) = take(key) {
            if key == "changes" {
                out.push(if body.is_empty() {
                    "(no visible change)".into()
                } else {
                    format!("changed:\n{body}")
                });
            } else {
                out.push(body);
            }
        }
    }
    for key in [
        "tabId", "scroll", "total", "ms", "accepted", "loaded", "base64",
    ] {
        let dropped = take(key);
        if key == "loaded" && dropped == Some(Value::Bool(false)) {
            out.push("(the page had not finished loading)".into());
        }
    }
    if let Some(next) = take("next") {
        out.push(format!("(more: pass offset {next})"));
    }
    if !rest.is_empty() {
        out.push(Value::Object(rest).to_string());
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn renders_like_the_mcp_server() {
        let text = json!({"title": "T", "url": "u", "text": "Hello", "next": 6000, "tabId": "t1"});
        assert_eq!(mcp_render(&text), "T — u\nHello\n(more: pass offset 6000)");
    }

    #[test]
    fn matches_python_json_dumps() {
        // python -c "import json; print(json.dumps({'outline':'a\nb', 'values':{'b':1,'a':'é'}, 'recent':[], 'x':[True,None,2.5,1.0]}, ensure_ascii=False))"
        let value = Py::Obj(vec![
            ("outline".into(), Py::Val(json!("a\nb"))),
            ("values".into(), Py::Val(json!({"b": 1, "a": "é"}))),
            ("recent".into(), Py::Val(json!([]))),
            ("x".into(), Py::Val(json!([true, null, 2.5, 1.0]))),
        ]);
        assert_eq!(
            dumps(&value),
            r#"{"outline": "a\nb", "values": {"a": "é", "b": 1}, "recent": [], "x": [true, null, 2.5, 1.0]}"#
        );
        assert_eq!(string("q\"\u{1}"), r#""q\"\u0001""#);
    }
}
