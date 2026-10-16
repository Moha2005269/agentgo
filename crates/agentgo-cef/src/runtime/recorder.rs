//! Workflow recording and replay.
//!
//! A session can record what it does (`workflow.record`) into a JSON-lines
//! file in the profile. Every step keeps the command, its parameters, whether
//! it worked and, for actions on a reference, the outline line of the target
//! (`button "Add to cart"`). Replay finds each target again by that line on
//! the page as it is now, so a recording survives a reload or a new session,
//! where reference numbers differ.
//!
//! The same files are the training data for Laya: a `decision.record` step
//! holds the full decision record (state and candidates) and the following
//! `decision.apply` step holds the choice and its outcome.

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;

use serde_json::{json, Value};

use super::sessions::{Fail, Reply};
use super::state;

struct Recording {
    name: String,
    paused: bool,
    steps: usize,
}

thread_local! {
    static ACTIVE: RefCell<HashMap<String, Recording>> = RefCell::new(HashMap::new());
}

/// Recordings live in AgentGo's durable data folder, so the recordings of an
/// agent's throwaway run survive it.
pub fn dir() -> PathBuf {
    state::data_dir().join("recordings")
}

pub fn file_of(name: &str) -> Result<PathBuf, Fail> {
    if name.is_empty()
        || name.len() > 64
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(Fail::params(
            "a recording name is 1 to 64 letters, digits, - or _",
        ));
    }
    Ok(dir().join(format!("{name}.jsonl")))
}

/// Whether `session` is recording right now (and not paused).
pub fn active(session: &str) -> bool {
    ACTIVE.with(|all| {
        all.borrow()
            .get(session)
            .is_some_and(|recording| !recording.paused)
    })
}

/// Commands that are not part of a recording: the recorder itself and
/// reading commands that change nothing (except the decision record, which
/// is what Laya will be trained on).
fn recorded(method: &str) -> bool {
    !method.starts_with("workflow.record")
        && !matches!(
            method,
            "workflow.pause" | "workflow.resume" | "workflow.replay" | "workflow.disable"
        )
        && (method == "decision.record" || !super::dispatch::is_read_only(method))
}

/// The target line of an action result, without a field's current value.
fn target_line(reply: &Value) -> Option<String> {
    reply
        .get("target")
        .or_else(|| reply.get("field"))
        .and_then(Value::as_str)
        .map(|line| line.split(" value=").next().unwrap_or(line).to_string())
        .filter(|line| !line.starts_with("point "))
}

/// Adds one finished command to the session's recording.
pub fn note(session: &str, method: &str, params: &Value, reply: &Reply) {
    if !recorded(method) {
        return;
    }
    let Some(name) = ACTIVE.with(|all| {
        all.borrow()
            .get(session)
            .filter(|r| !r.paused)
            .map(|r| r.name.clone())
    }) else {
        return;
    };
    let mut params = params.clone();
    let (ok, result) = match reply {
        Ok(value) => (true, value.clone()),
        Err(fail) => (
            false,
            json!({"error": format!("{:?}", fail.code), "message": fail.message}),
        ),
    };
    let target = target_line(&result);
    // Text typed into a password field is never written down; replay then
    // stops there and the agent signs in with credentials.fill.
    if target
        .as_deref()
        .is_some_and(|line| line.contains("(password)"))
    {
        if let Some(map) = params.as_object_mut() {
            map.insert("text".into(), Value::Null);
            map.insert("redacted".into(), json!(true));
        }
    }
    let step = json!({
        "at": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64),
        "method": method,
        "params": params,
        "ok": ok,
        "target": target,
        // Decision records are kept whole (training data); other results are
        // kept short.
        "result": if method.starts_with("decision.") { result } else { json!({"url": result.get("url"), "navigated": result.get("navigated"), "error": result.get("error")}) },
    });
    let Ok(file) = file_of(&name) else { return };
    let written = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&file)
        .and_then(|mut out| writeln!(out, "{step}"));
    if written.is_ok() {
        ACTIVE.with(|all| {
            if let Some(recording) = all.borrow_mut().get_mut(session) {
                recording.steps += 1;
            }
        });
    }
}

/// `workflow.record {name}` starts, `{stop:true}` finishes.
pub fn record(session: &str, params: &Value) -> Reply {
    if params["stop"] == true {
        let done = ACTIVE
            .with(|all| all.borrow_mut().remove(session))
            .ok_or_else(|| Fail::params("this session is not recording"))?;
        return Ok(
            json!({"name": done.name, "steps": done.steps, "file": file_of(&done.name)?.display().to_string(), "recording": false}),
        );
    }
    let name = params["name"]
        .as_str()
        .ok_or_else(|| Fail::params("workflow.record needs params.name (or stop:true)"))?;
    let file = file_of(name)?;
    std::fs::create_dir_all(dir()).map_err(|error| Fail::internal(error.to_string()))?;
    // A new recording replaces an old one of the same name.
    let _ = std::fs::remove_file(&file);
    ACTIVE.with(|all| {
        all.borrow_mut().insert(
            session.to_string(),
            Recording {
                name: name.to_string(),
                paused: false,
                steps: 0,
            },
        )
    });
    Ok(json!({"name": name, "recording": true, "file": file.display().to_string()}))
}

/// `workflow.pause` / `workflow.resume`.
pub fn set_paused(session: &str, paused: bool) -> Reply {
    ACTIVE.with(|all| {
        let mut all = all.borrow_mut();
        let recording = all
            .get_mut(session)
            .ok_or_else(|| Fail::params("this session is not recording"))?;
        recording.paused = paused;
        Ok(json!({"name": recording.name, "paused": paused, "steps": recording.steps}))
    })
}

/// `workflow.disable {name}`: keeps the file but refuses to replay it.
pub fn disable(params: &Value) -> Reply {
    let name = params["name"]
        .as_str()
        .ok_or_else(|| Fail::params("params.name is required"))?;
    let file = file_of(name)?;
    std::fs::rename(&file, file.with_extension("disabled"))
        .map_err(|_| Fail::params(format!("there is no recording named {name}")))?;
    Ok(json!({"name": name, "disabled": true}))
}

/// The steps of a recording that replay runs, in order.
pub fn steps(params: &Value) -> Result<Vec<Value>, Fail> {
    let name = params["name"]
        .as_str()
        .ok_or_else(|| Fail::params("workflow.replay needs params.name"))?;
    let file = file_of(name)?;
    if file.with_extension("disabled").exists() && !file.exists() {
        return Err(Fail::new(
            agentgo_protocol::ErrorCode::PolicyDenied,
            format!("the recording {name} is disabled"),
        ));
    }
    let text = std::fs::read_to_string(&file)
        .map_err(|_| Fail::params(format!("there is no recording named {name}")))?;
    Ok(text
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|step| {
            step["ok"] == true
                && !step["method"]
                    .as_str()
                    .unwrap_or("")
                    .starts_with("decision.")
        })
        .collect())
}

/// Finds the reference of `line` in a fresh outline.
pub fn find_ref(outline: &str, line: &str) -> Option<u64> {
    outline.lines().find_map(|candidate| {
        let (reference, rest) = candidate.strip_prefix('[')?.split_once("] ")?;
        (rest.split(" value=").next() == Some(line))
            .then(|| reference.parse().ok())
            .flatten()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_are_found_again_by_their_line() {
        let outline = "# Shop\n[40] button \"Add to cart\"\n[41] textbox \"Email\" value=\"old\"\n[42] link \"Help\"";
        assert_eq!(find_ref(outline, "button \"Add to cart\""), Some(40));
        assert_eq!(find_ref(outline, "textbox \"Email\""), Some(41));
        assert_eq!(find_ref(outline, "button \"Gone\""), None);
        assert_eq!(
            target_line(&json!({"field": "textbox \"Email\" value=\"x\""})).as_deref(),
            Some("textbox \"Email\"")
        );
        assert_eq!(target_line(&json!({"target": "point 4,5"})), None);
        assert!(file_of("../x").is_err() && file_of("").is_err());
    }
}
