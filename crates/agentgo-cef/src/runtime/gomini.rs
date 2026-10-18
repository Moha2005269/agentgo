//! GoMini in the browser: the person's download and switch, the resident
//! worker, and `gomini.delegate`, where GoMini takes the routine steps of
//! one subgoal so the agent's large model does not read every page.
//!
//! ```text
//! agent: gomini.delegate {subgoal, values, completion}
//!   loop (budget, deadline, Pause/Stop):
//!     decision.record -> shortlist -> gomini.agentgo/1 -> GoMini
//!       -> independent gate -> decision.apply -> read back
//!   -> {status: done | needs_help | stopped, steps, evidence}
//! ```
//!
//! The model only ever names one of the offered keys. What runs is decided
//! by the gate and the browser's own policy, never by its probability.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use agentgo_gomini::{gate, install, pack, shortlist, Layout, Server};
use agentgo_protocol::ErrorCode;
use serde_json::{json, Value};

use super::sessions::{self, Caller, Control, Fail, Reply};
use super::{dispatch, exec, page, state};

/// The installation job, read by Settings while it runs.
#[derive(Default)]
struct Job {
    running: bool,
    cancel: Option<Arc<AtomicBool>>,
    progress: Value,
    error: Option<String>,
}

/// The worker and why it is not running, if it is not.
#[derive(Default)]
struct Worker {
    server: Option<Arc<Server>>,
    starting: bool,
    error: Option<String>,
}

static JOB: Mutex<Option<Job>> = Mutex::new(None);
static WORKER: Mutex<Option<Worker>> = Mutex::new(None);

fn job<T>(f: impl FnOnce(&mut Job) -> T) -> T {
    let mut slot = JOB.lock().unwrap_or_else(|e| e.into_inner());
    f(slot.get_or_insert_with(Job::default))
}

fn worker<T>(f: impl FnOnce(&mut Worker) -> T) -> T {
    let mut slot = WORKER.lock().unwrap_or_else(|e| e.into_inner());
    f(slot.get_or_insert_with(Worker::default))
}

/// Where GoMini lives: `%LOCALAPPDATA%\AgentGo\gomini` (tests may point
/// `AGENTGO_GOMINI_ROOT` elsewhere).
pub fn layout() -> Layout {
    let root = std::env::var_os("AGENTGO_GOMINI_ROOT")
        .map(std::path::PathBuf::from)
        .or_else(Layout::default_root)
        .unwrap_or_else(|| state::data_dir().join("gomini"));
    Layout::new(root)
}

/// The person's switch.
pub fn enabled() -> bool {
    state::settings_snapshot()["gominiEnabled"] == true
}

/// Switched on and installed.
pub fn active() -> bool {
    enabled() && install::installed(&layout()).is_some()
}

/// What Settings, `decision.status` and agents are told.
pub fn status() -> Value {
    let layout = layout();
    let installed = install::installed(&layout);
    let (running, starting, worker_error) =
        worker(|w| (w.server.is_some(), w.starting, w.error.clone()));
    let download = job(|j| {
        json!({
            "running": j.running,
            "progress": j.progress,
            "error": j.error,
        })
    });
    json!({
        "model": agentgo_gomini::artifacts::MODEL_NAME,
        "installed": installed.is_some(),
        "enabled": enabled(),
        "active": enabled() && installed.is_some(),
        "running": running,
        "starting": starting,
        "error": worker_error,
        "installation": installed.as_ref().map(install::Installed::to_json),
        "downloadBytes": install::download_size(),
        "download": download,
        "folder": layout.root.display().to_string(),
    })
}

/// Starts the download in the background (Settings polls `gomini.status`).
pub fn start_install() -> Result<Value, Fail> {
    let cancel = Arc::new(AtomicBool::new(false));
    let already = job(|j| {
        if j.running {
            return true;
        }
        *j = Job {
            running: true,
            cancel: Some(cancel.clone()),
            progress: json!({"stage": "start", "text": "Starting", "done": 0, "total": install::download_size()}),
            error: None,
        };
        false
    });
    if already {
        return Ok(status());
    }
    let layout = layout();
    std::thread::spawn(move || {
        let result = install::install(&layout, &cancel, &|p| {
            job(|j| {
                j.progress = json!({"stage": p.stage, "text": p.text, "done": p.done, "total": p.total})
            })
        });
        let ok = result.is_ok();
        job(|j| {
            j.running = false;
            j.cancel = None;
            j.error = result.err();
        });
        exec::post(move || {
            if ok {
                // Downloading it is the person saying yes: switch it on.
                let _ = state::update_settings(&json!({"gominiEnabled": true}));
                state::log_event("AG-GOMINI installed");
                warm_up();
            }
            state::notify_shell_state();
        });
    });
    Ok(status())
}

/// Stops a running download (partial files stay, so a retry resumes).
pub fn cancel_install() -> Value {
    job(|j| {
        if let Some(cancel) = &j.cancel {
            cancel.store(true, Ordering::SeqCst);
        }
    });
    status()
}

/// Stops the worker and deletes everything GoMini downloaded.
pub fn remove() -> Reply {
    if job(|j| j.running) {
        return Err(Fail::params("cancel the download first"));
    }
    stop_worker();
    let _ = state::update_settings(&json!({"gominiEnabled": false}));
    install::remove(&layout()).map_err(Fail::internal)?;
    Ok(status())
}

/// Stops the worker (the switch went off, or AgentGo is closing).
pub fn stop_worker() {
    worker(|w| {
        w.server = None;
        w.error = None;
    });
}

/// Starts the worker in the background if GoMini is active, so the first
/// delegated subgoal does not wait for the model to load.
pub fn warm_up() {
    if !active() || worker(|w| w.server.is_some() || w.starting) {
        return;
    }
    worker(|w| w.starting = true);
    let layout = layout();
    std::thread::spawn(move || {
        let started = install::installed(&layout)
            .ok_or_else(|| "GoMini is not installed".to_string())
            .and_then(|found| Server::start(&layout, &found));
        worker(|w| {
            w.starting = false;
            match started {
                Ok(server) => {
                    w.server = Some(Arc::new(server));
                    w.error = None;
                }
                Err(e) => w.error = Some(e),
            }
        });
    });
}

/// The running worker, starting (and waiting for) it when needed.
async fn server() -> Result<Arc<Server>, Fail> {
    if !enabled() {
        return Err(Fail::new(
            ErrorCode::CapabilityUnsupported,
            "GoMini is switched off; the person turns it on in Settings > GoMini",
        ));
    }
    if install::installed(&layout()).is_none() {
        return Err(Fail::new(
            ErrorCode::CapabilityUnsupported,
            "GoMini is not downloaded; the person downloads it in Settings > GoMini",
        ));
    }
    // A worker that died is replaced.
    if let Some(server) = worker(|w| w.server.clone()) {
        let alive = exec::blocking(3_000, {
            let server = server.clone();
            move || server.healthy()
        })
        .await
        .unwrap_or(false);
        if alive {
            return Ok(server);
        }
        worker(|w| w.server = None);
    }
    warm_up();
    let deadline = Instant::now() + Duration::from_secs(95);
    loop {
        let (server, starting, error) = worker(|w| (w.server.clone(), w.starting, w.error.clone()));
        if let Some(server) = server {
            return Ok(server);
        }
        if !starting {
            return Err(Fail::internal(error.unwrap_or_else(|| "GoMini worker did not start".into())));
        }
        if Instant::now() > deadline {
            return Err(Fail::new(ErrorCode::Timeout, "GoMini worker did not start in time"));
        }
        exec::sleep(100).await;
    }
}

async fn call(caller: &Caller, method: &str, params: Value) -> Reply {
    Box::pin(dispatch::run(caller, method, params)).await
}

fn origin(url: &str) -> String {
    let after = url.find("://").map_or(0, |i| i + 3);
    let end = url[after..].find('/').map_or(url.len(), |i| after + i);
    url[..end].to_ascii_lowercase()
}

/// The page's readable text as GoMini was trained with it: `observe.text`
/// as the MCP server renders it (`title — url`, then the text).
async fn page_text(caller: &Caller, tab: &str) -> String {
    match call(caller, "observe.text", json!({"tabId": tab})).await {
        Ok(Value::String(s)) => s,
        Ok(other) => agentgo_gomini::pyjson::mcp_render(&other),
        Err(_) => String::new(),
    }
}

/// The text cut to `chars` (1,400 in training), with the page's status
/// lines added when they are not in it.
fn cut_text(text: &str, chars: usize, status: &[String]) -> String {
    let mut out: String = text.chars().take(chars).collect();
    let joined = status.join(" ");
    if !joined.is_empty() && !out.contains(&joined) {
        out.push_str("\nStatus: ");
        out.push_str(&joined);
    }
    out
}

/// With `AGENTGO_GOMINI_TRACE=<file>`, every request GoMini saw and its
/// answer is appended as one JSON line (debugging and training data).
fn trace(body: &str, outcome: &Result<pack::Choice, String>) {
    let Some(path) = std::env::var_os("AGENTGO_GOMINI_TRACE") else {
        return;
    };
    let answer = match outcome {
        Ok(choice) => json!({"choice": choice.choice, "probabilities": choice.probabilities, "inputTokens": choice.input_tokens}),
        Err(e) => json!({"error": e}),
    };
    let line = json!({"request": serde_json::from_str::<Value>(body).unwrap_or(Value::Null), "requestText": body, "answer": answer});
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        use std::io::Write;
        let _ = writeln!(file, "{line}");
    }
}

/// One delegated subgoal, run by GoMini within a budget.
pub async fn delegate(caller: &Caller, params: &Value) -> Reply {
    let tab = page::tab_of(caller, params)?;
    let subgoal = params["subgoal"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| Fail::params("gomini.delegate needs params.subgoal: one small step in plain words"))?;
    let max_steps = params["maxSteps"].as_u64().unwrap_or(8).clamp(1, 12);
    let deadline_ms = params["deadlineMs"].as_u64().unwrap_or(60_000).clamp(5_000, 180_000);
    let started = Instant::now();
    let deadline = started + Duration::from_millis(deadline_ms);
    let server = server().await?;
    let record_params = json!({
        "tabId": tab,
        "subgoal": subgoal,
        "values": params.get("values").cloned().unwrap_or_else(|| json!({})),
        "completion": params.get("completion"),
        "constraints": params.get("constraints").cloned().unwrap_or_else(|| json!([])),
        "max": 200,
    });
    let mut steps: Vec<Value> = Vec::new();
    let mut recent: Vec<Value> = Vec::new();
    let mut start_origin: Option<String> = None;
    let mut last: Option<(String, String)> = None;
    // Supplied value name -> the field GoMini put it in.
    let mut bound: Vec<(String, u64)> = Vec::new();
    let mut model_ms = 0u128;
    let finish = |status: &str, reason: Option<String>, record: &Value, steps: &[Value], model_ms: u128| {
        let outline: String = record["state"]["outline"]
            .as_str()
            .unwrap_or_default()
            .chars()
            .take(600)
            .collect();
        json!({
            "status": status,
            "reason": reason,
            "verified": false,
            "attempts": steps.len(),
            "steps": steps,
            "lastChoice": steps.last().map(|s| s["choice"].clone()),
            "url": record["state"]["url"],
            "title": record["state"]["title"],
            "evidence": {"status": record["state"]["status"], "outline": outline},
            "model": agentgo_gomini::artifacts::MODEL_NAME,
            "modelMs": model_ms as u64,
            "ms": started.elapsed().as_millis() as u64,
        })
    };
    let mut record = Value::Null;
    for step in 0..max_steps {
        while sessions::control() == Control::Paused && Instant::now() < deadline {
            exec::sleep(200).await;
        }
        let stopped = sessions::control() == Control::Stopped;
        if Instant::now() >= deadline {
            return Ok(finish("needs_help", Some("the deadline passed".into()), &record, &steps, model_ms));
        }
        record = call(caller, "decision.record", record_params.clone()).await?;
        let url = record["state"]["url"].as_str().unwrap_or_default().to_string();
        let here = origin(&url);
        let origin_ok = start_origin.get_or_insert_with(|| here.clone()) == &here;
        let status_lines: Vec<String> = record["state"]["status"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|s| s.as_str().map(str::to_string))
            .collect();
        let executor = pack::Executor {
            cancelled: stopped,
            attempts_remaining: max_steps - step,
        };
        let full_text = page_text(caller, &tab).await;
        // What each supplied value's field holds now, so GoMini can tell a
        // finished form from an unfinished one (as in its form training).
        let readback = match record["values"].as_object() {
            Some(values) if !values.is_empty() => {
                let mut fields = Vec::new();
                for name in values.keys() {
                    let held = match bound.iter().find(|(n, _)| n == name) {
                        Some((_, reference)) => call(caller, "runtime.get_value", json!({"tabId": tab, "ref": reference}))
                            .await
                            .map(|v| v["value"].clone())
                            .unwrap_or(json!("")),
                        None => json!(""),
                    };
                    fields.push((name.clone(), held));
                }
                Some(fields)
            }
            _ => None,
        };
        // Narrow the page, then compact further if the input is still too large.
        let mut outcome = Err(String::new());
        let mut attempt_actions = Vec::new();
        let mut attempt_candidates = Vec::new();
        for (keep, outline_chars, text_chars) in [(16, 1500, 1400), (10, 700, 700), (6, 300, 300)] {
            let short = shortlist::shortlist(&record, keep, outline_chars);
            let text = cut_text(&full_text, text_chars, &status_lines);
            let (request, actions) = match pack::from_record(&short, &text, &recent, &executor, readback.clone()) {
                Ok(built) => built,
                Err(e) => {
                    outcome = Err(e);
                    continue;
                }
            };
            let asked = Instant::now();
            let body = pack::body(&request);
            outcome = exec::blocking(40_000, {
                let server = server.clone();
                move || agentgo_gomini::choose(&server, &request)
            })
            .await
            .unwrap_or_else(|| Err("GoMini took too long".into()));
            trace(&body, &outcome);
            model_ms += asked.elapsed().as_millis();
            attempt_actions = actions;
            attempt_candidates = short["candidates"].as_array().cloned().unwrap_or_default();
            match &outcome {
                Ok(_) => break,
                Err(e) if e.contains("limit") || e.contains("exceed") || e.contains("longer") || e.contains("narrow") => continue,
                Err(_) => break,
            }
        }
        let choice = match outcome {
            Ok(choice) => choice,
            Err(e) => {
                return Ok(finish("needs_help", Some(format!("GoMini could not take this page: {e}")), &record, &steps, model_ms))
            }
        };
        let verdict = gate::gate(&choice.choice, &attempt_actions, &attempt_candidates, &executor, step, origin_ok);
        let action = attempt_actions
            .iter()
            .find(|(key, _)| key == &choice.choice)
            .map(|(_, a)| a.clone());
        let mut entry = json!({
            "choice": choice.choice,
            "p": (choice.top_probability * 1000.0).round() / 1000.0,
            "verdict": format!("{verdict:?}"),
        });
        match verdict {
            gate::Verdict::Stop => {
                steps.push(entry);
                return Ok(finish("stopped", Some(if stopped { "the person pressed Stop".into() } else { "budget used".into() }), &record, &steps, model_ms));
            }
            gate::Verdict::Escalate(reason) => {
                steps.push(entry);
                return Ok(finish("needs_help", Some(reason), &record, &steps, model_ms));
            }
            gate::Verdict::Done => {
                steps.push(entry);
                return Ok(finish("done", None, &record, &steps, model_ms));
            }
            gate::Verdict::Wait => {
                let _ = call(caller, "wait.settle", json!({"tabId": tab})).await;
                entry["ok"] = json!(true);
                recent.push(json!({"choice": choice.choice, "ok": true}));
            }
            gate::Verdict::Apply => {
                let Some(action) = action else {
                    return Ok(finish("needs_help", Some("no action for the choice".into()), &record, &steps, model_ms));
                };
                let revision = record["state"]["revision"].as_str().unwrap_or_default().to_string();
                if last.as_ref() == Some(&(choice.choice.clone(), revision.clone())) {
                    steps.push(entry);
                    return Ok(finish("needs_help", Some("the same step on an unchanged page: no progress".into()), &record, &steps, model_ms));
                }
                last = Some((choice.choice.clone(), revision));
                let applied = call(
                    caller,
                    "decision.apply",
                    json!({"tabId": tab, "choose": action.choose, "value": action.value}),
                )
                .await;
                let mut ok = applied.is_ok();
                entry["ran"] = applied.as_ref().map(|r| r["ran"].clone()).unwrap_or(Value::Null);
                if let Err(fail) = &applied {
                    entry["error"] = json!(fail.message);
                    // The browser's own gates (approval, sign-in, a human
                    // check) are the planner's to handle, not GoMini's.
                    if matches!(fail.code, ErrorCode::ApprovalRequired | ErrorCode::HumanHandoff | ErrorCode::PolicyDenied | ErrorCode::TargetBlocked) {
                        steps.push(entry);
                        return Ok(finish("needs_help", Some(fail.message.clone()), &record, &steps, model_ms));
                    }
                }
                // Typing is verified by reading the field back.
                if ok {
                    if let (Some(name), Some(reference)) = (
                        action.value.as_deref(),
                        attempt_candidates
                            .iter()
                            .find(|c| c["id"] == action.choose.as_str())
                            .and_then(|c| c["ref"].as_u64()),
                    ) {
                        let expected = params["values"][name].as_str().unwrap_or_default();
                        let read = call(caller, "runtime.get_value", json!({"tabId": tab, "ref": reference}))
                            .await
                            .map(|v| v["value"].clone())
                            .unwrap_or(Value::Null);
                        ok = read.as_str() == Some(expected)
                            || attempt_candidates.iter().any(|c| c["id"] == action.choose.as_str() && c["op"] == "select");
                        entry["readback"] = json!(ok);
                        bound.retain(|(n, r)| n != name && *r != reference);
                        bound.push((name.to_string(), reference));
                    }
                }
                entry["ok"] = json!(ok);
                recent.push(json!({"choice": choice.choice, "ok": ok}));
                if !ok && steps.iter().rev().take(1).any(|s| s["ok"] == false) {
                    steps.push(entry);
                    return Ok(finish("needs_help", Some("two steps in a row failed".into()), &record, &steps, model_ms));
                }
            }
        }
        steps.push(entry);
    }
    Ok(finish("needs_help", Some(format!("the step budget ({max_steps}) is used")), &record, &steps, model_ms))
}

/// Protocol commands for GoMini.
pub async fn command(caller: &Caller, method: &str, params: &Value) -> Reply {
    let operator = || {
        if caller.is_shell() {
            Ok(())
        } else {
            Err(Fail::new(
                ErrorCode::PolicyDenied,
                format!("{method}: downloading and removing GoMini is the person's decision"),
            ))
        }
    };
    match method {
        "gomini.status" => Ok(status()),
        "gomini.install" => {
            operator()?;
            start_install()
        }
        "gomini.cancel" => {
            operator()?;
            Ok(cancel_install())
        }
        "gomini.remove" => {
            operator()?;
            remove()
        }
        "gomini.delegate" => delegate(caller, params).await,
        _ => Err(Fail::new(ErrorCode::MethodNotFound, method.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origins_compare_scheme_and_host() {
        assert_eq!(origin("https://Shop.example/a/b?c"), "https://shop.example");
        assert_eq!(origin("http://127.0.0.1:5000"), "http://127.0.0.1:5000");
        assert_ne!(origin("https://shop.example/x"), origin("https://evil.example/x"));
    }
}
