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
use super::{decision, dispatch, exec, page, state};

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

/// Where GoMini lives: `gomini` in AgentGo's data folder
/// (`%LOCALAPPDATA%\AgentGo`, or `AGENTGO_DATA_ROOT`), where AgentGo Setup
/// puts it too. Tests may point `AGENTGO_GOMINI_ROOT` elsewhere.
pub fn layout() -> Layout {
    let root = std::env::var_os("AGENTGO_GOMINI_ROOT")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("AGENTGO_DATA_ROOT")
                .map(|base| std::path::PathBuf::from(base).join("gomini"))
        })
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
                j.progress =
                    json!({"stage": p.stage, "text": p.text, "done": p.done, "total": p.total})
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
            return Err(Fail::internal(
                error.unwrap_or_else(|| "GoMini worker did not start".into()),
            ));
        }
        if Instant::now() > deadline {
            return Err(Fail::new(
                ErrorCode::Timeout,
                "GoMini worker did not start in time",
            ));
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
        Ok(choice) => {
            json!({"choice": choice.choice, "probabilities": choice.probabilities, "inputTokens": choice.input_tokens})
        }
        Err(e) => json!({"error": e}),
    };
    let line = json!({"request": serde_json::from_str::<Value>(body).unwrap_or(Value::Null), "requestText": body, "answer": answer});
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
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
        .ok_or_else(|| {
            Fail::params("gomini.delegate needs params.subgoal: one small step in plain words")
        })?;
    let max_steps = params["maxSteps"].as_u64().unwrap_or(8).clamp(1, 12);
    let deadline_ms = params["deadlineMs"]
        .as_u64()
        .unwrap_or(60_000)
        .clamp(5_000, 180_000);
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
    // Steps that ran and checked out.
    let mut steps_ran = 0usize;
    let mut model_ms = 0u128;
    let finish =
        |status: &str, reason: Option<String>, record: &Value, steps: &[Value], model_ms: u128| {
            let outline: String = record["state"]["outline"]
                .as_str()
                .unwrap_or_default()
                .chars()
                .take(600)
                .collect();
            // Compact for the planner: each step as `choice` (failures say why),
            // and the page excerpt only when it has to look itself.
            let compact: Vec<Value> = steps
                .iter()
                .map(|s| match (s["ok"].as_bool(), s.get("error")) {
                    // Chosen but left for the planner (a send, a submit).
                    _ if s["held"] == true => json!({"choice": s["choice"], "held": true}),
                    (_, Some(error)) => json!({"choice": s["choice"], "error": error}),
                    (Some(false), None) => json!({"choice": s["choice"], "ok": false}),
                    _ => s["choice"].clone(),
                })
                .collect();
            let mut evidence = json!({"status": record["state"]["status"]});
            if status != "done" {
                evidence["outline"] = json!(outline);
            }
            json!({
                "status": status,
                "reason": reason,
                "verified": false,
                "steps": compact,
                "url": record["state"]["url"],
                "title": record["state"]["title"],
                "evidence": evidence,
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
            return Ok(finish(
                "needs_help",
                Some("the deadline passed".into()),
                &record,
                &steps,
                model_ms,
            ));
        }
        record = call(caller, "decision.record", record_params.clone()).await?;
        let url = record["state"]["url"]
            .as_str()
            .unwrap_or_default()
            .to_string();
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
        // A subgoal that names its boxes in quotes ("Check "Hhc" and "4jU"")
        // is done when exactly those are ticked: say so instead of asking.
        let named = named_boxes(subgoal, &record);
        if !named.is_empty() && named.iter().all(|(_, _, checked)| *checked) {
            return Ok(finish(
                "done",
                Some("every box the subgoal names is checked".into()),
                &record,
                &steps,
                model_ms,
            ));
        }
        let full_text = page_text(caller, &tab).await;
        // What each supplied value's field holds now, so GoMini can tell a
        // finished form from an unfinished one (as in its form training).
        let readback = match record["values"].as_object() {
            Some(values) if !values.is_empty() => {
                let mut fields = Vec::new();
                for name in values.keys() {
                    let expected = params["values"][name].as_str().unwrap_or_default();
                    let held = match bound.iter().find(|(n, _)| n == name) {
                        // A field that holds the value in its own format (a
                        // date picker) is shown as holding it, as supplied.
                        Some((_, reference))
                            if holds(&tab, *reference, expected).await == Ok(true) =>
                        {
                            json!(expected)
                        }
                        Some((_, reference)) => call(
                            caller,
                            "runtime.get_value",
                            json!({"tabId": tab, "ref": reference}),
                        )
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
            let (request, actions) =
                match pack::from_record(&short, &text, &recent, &executor, readback.clone()) {
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
                Err(e)
                    if e.contains("limit")
                        || e.contains("exceed")
                        || e.contains("longer")
                        || e.contains("narrow") =>
                {
                    continue
                }
                Err(_) => break,
            }
        }
        let mut choice = match outcome {
            Ok(choice) => choice,
            Err(e) => {
                return Ok(finish(
                    "needs_help",
                    Some(format!("GoMini could not take this page: {e}")),
                    &record,
                    &steps,
                    model_ms,
                ))
            }
        };
        // Supplied values come first: while one is not in its field, a click
        // (Search, Submit), a planner's step or "done" is premature. Take
        // GoMini's most likely step that fills a missing value, if the gate
        // would allow it (it never offers a step the page does not have).
        let missing_now: Vec<String> = readback
            .as_ref()
            .map(|fields| {
                fields
                    .iter()
                    .filter(|(name, held)| held.as_str() != params["values"][name].as_str())
                    .map(|(name, _)| name.clone())
                    .collect()
            })
            .unwrap_or_default();
        let fills_missing = |key: &str| {
            attempt_actions
                .iter()
                .find(|(k, _)| k == key)
                .and_then(|(_, a)| a.value.as_ref())
                .is_some_and(|v| missing_now.contains(v))
        };
        let mut overridden = None;
        if !missing_now.is_empty() && !fills_missing(&choice.choice) {
            let best = choice
                .probabilities
                .iter()
                .filter(|(key, _)| fills_missing(key))
                .filter(|(key, _)| {
                    gate::gate(
                        key,
                        &attempt_actions,
                        &attempt_candidates,
                        &executor,
                        step,
                        origin_ok,
                    ) == gate::Verdict::Apply
                })
                .max_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(key, _)| key.clone());
            if let Some(best) = best {
                overridden = Some(std::mem::replace(&mut choice.choice, best));
            }
        }
        // A value already in its field is not typed into a second one (it
        // filled the same number into every textbox): take GoMini's best step
        // that does not.
        let placed: Vec<(String, u64)> = bound
            .iter()
            .filter(|(name, _)| !missing_now.contains(name))
            .cloned()
            .collect();
        let refills = |key: &str| {
            let Some((_, action)) = attempt_actions.iter().find(|(k, _)| k == key) else {
                return false;
            };
            let target = attempt_candidates
                .iter()
                .find(|c| c["id"] == action.choose.as_str())
                .and_then(|c| c["ref"].as_u64());
            action.value.as_ref().is_some_and(|value| {
                placed
                    .iter()
                    .any(|(name, reference)| name == value && Some(*reference) != target)
            })
        };
        if refills(&choice.choice) {
            let best = choice
                .probabilities
                .iter()
                .filter(|(key, _)| !refills(key))
                .max_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(key, _)| key.clone());
            if let Some(best) = best {
                let first = std::mem::replace(&mut choice.choice, best);
                overridden.get_or_insert(first);
            }
        }
        // With named boxes, a box step is one of them that is not ticked yet;
        // GoMini's most likely such step is taken instead of another box.
        let is_box = |key: &str| {
            attempt_candidates.iter().any(|c| {
                c["id"] == key
                    && c["target"]
                        .as_str()
                        .is_some_and(|t| t.starts_with("checkbox "))
            })
        };
        let wanted = |key: &str| named.iter().any(|(id, _, checked)| id == key && !checked);
        if !named.is_empty() && is_box(&choice.choice) && !wanted(&choice.choice) {
            let best = choice
                .probabilities
                .iter()
                .filter(|(key, _)| wanted(key) && attempt_actions.iter().any(|(k, _)| k == key))
                .max_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(key, _)| key.clone());
            if let Some(best) = best {
                let first = std::mem::replace(&mut choice.choice, best);
                overridden.get_or_insert(first);
            }
        }
        // "Done" before any step ran is never true for a subgoal with nothing
        // to type (it is something to do): GoMini's most likely step on the
        // page is taken instead, when it is a real contender (20% or more).
        let nothing_to_type = params["values"].as_object().is_none_or(|v| v.is_empty());
        if choice.choice == "done" && steps_ran == 0 && nothing_to_type {
            let best = choice
                .probabilities
                .iter()
                .filter(|(key, p)| {
                    key.starts_with('r')
                        && *p >= 0.2
                        && attempt_actions.iter().any(|(k, _)| k == key)
                })
                .max_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(key, _)| key.clone());
            if let Some(best) = best {
                let first = std::mem::replace(&mut choice.choice, best);
                overridden.get_or_insert(first);
            }
        }
        let verdict = gate::gate(
            &choice.choice,
            &attempt_actions,
            &attempt_candidates,
            &executor,
            step,
            origin_ok,
        );
        let action = attempt_actions
            .iter()
            .find(|(key, _)| key == &choice.choice)
            .map(|(_, a)| a.clone());
        let mut entry = json!({
            "choice": choice.choice,
            "p": (choice.top_probability * 1000.0).round() / 1000.0,
            "verdict": format!("{verdict:?}"),
        });
        if let Some(first) = &overridden {
            entry["instead_of"] = json!(first);
        }
        match verdict {
            gate::Verdict::Stop => {
                steps.push(entry);
                return Ok(finish(
                    "stopped",
                    Some(if stopped {
                        "the person pressed Stop".into()
                    } else {
                        "budget used".into()
                    }),
                    &record,
                    &steps,
                    model_ms,
                ));
            }
            gate::Verdict::Escalate(reason) => {
                if reason.ends_with("is a step for the planner, not GoMini") {
                    entry["held"] = json!(true);
                }
                steps.push(entry);
                // Every supplied value is in place and what is left (submit,
                // pay, sign in) is the planner's: the subgoal is done.
                // ...and with nothing to type, once a step ran (a section was
                // opened and Submit is next).
                let has_values = params["values"].as_object().is_some_and(|v| !v.is_empty());
                let filled = reason.ends_with("is a step for the planner, not GoMini")
                    && if has_values {
                        unfilled(&tab, params, &bound).await.is_empty()
                    } else {
                        steps_ran > 0
                    };
                if filled {
                    // The planner gets the step with its reference, so it can
                    // press it without reading the page again.
                    let next = attempt_candidates
                        .iter()
                        .find(|c| c["id"] == choice.choice.as_str())
                        .and_then(|c| {
                            Some(format!(
                                "[{}] {}",
                                c["ref"].as_u64()?,
                                c["target"].as_str()?
                            ))
                        })
                        .unwrap_or_else(|| {
                            reason
                                .trim_end_matches(" is a step for the planner, not GoMini")
                                .to_string()
                        });
                    return Ok(finish(
                        "done",
                        Some(format!(
                            "{} the next step ({next}) is yours",
                            if has_values {
                                "every value is filled in;"
                            } else {
                                "the steps ran;"
                            }
                        )),
                        &record,
                        &steps,
                        model_ms,
                    ));
                }
                return Ok(finish(
                    "needs_help",
                    Some(reason),
                    &record,
                    &steps,
                    model_ms,
                ));
            }
            gate::Verdict::Done => {
                // "Done" holds only when every supplied value is in its field,
                // and, for a subgoal with no values, only after a step ran:
                // a subgoal is something to do, so "done" before doing
                // anything means GoMini found nothing it could do.
                let missing = unfilled(&tab, params, &bound).await;
                let has_values = params["values"].as_object().is_some_and(|v| !v.is_empty());
                if missing.is_empty() && (has_values || steps_ran > 0) {
                    steps.push(entry);
                    // The subgoal ended with a final step ("...and send it")
                    // that GoMini leaves to the planner: say which control.
                    let reason = final_step(subgoal, &record, &steps).map(|line| {
                        format!(
                            "the values are in place; {line} was not pressed: that step is yours"
                        )
                    });
                    return Ok(finish("done", reason, &record, &steps, model_ms));
                }
                let why = if missing.is_empty() {
                    "not done: no step was taken yet".to_string()
                } else {
                    format!("not done: {} not filled yet", missing.join(", "))
                };
                entry["ok"] = json!(false);
                entry["error"] = json!(why);
                steps.push(entry);
                let again = recent.last().is_some_and(|r| r["choice"] == choice.choice);
                recent.push(json!({"choice": choice.choice, "ok": false}));
                if again || step + 1 == max_steps {
                    return Ok(finish(
                        "needs_help",
                        Some(if missing.is_empty() {
                            "GoMini found no step to take toward this subgoal; read the page and act yourself or give a narrower subgoal".to_string()
                        } else {
                            format!(
                                "GoMini called it done, but these values are not in the page yet: {}",
                                missing.join(", ")
                            )
                        }),
                        &record,
                        &steps,
                        model_ms,
                    ));
                }
                continue;
            }
            gate::Verdict::Wait => {
                let _ = call(caller, "wait.settle", json!({"tabId": tab})).await;
                entry["ok"] = json!(true);
                recent.push(json!({"choice": choice.choice, "ok": true}));
            }
            gate::Verdict::Apply => {
                let Some(action) = action else {
                    return Ok(finish(
                        "needs_help",
                        Some("no action for the choice".into()),
                        &record,
                        &steps,
                        model_ms,
                    ));
                };
                let revision = record["state"]["revision"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                if last.as_ref() == Some(&(choice.choice.clone(), revision.clone())) {
                    let target = attempt_candidates
                        .iter()
                        .find(|c| c["id"] == choice.choice.as_str())
                        .and_then(|c| c["target"].as_str())
                        .unwrap_or("it")
                        .to_string();
                    steps.push(entry);
                    // The subgoal named this very control in quotes ("Click
                    // the "Next" button") and the click ran: that was the job.
                    let label = target
                        .split_once('"')
                        .and_then(|(_, rest)| rest.rsplit_once('"'))
                        .map(|(label, _)| label.trim().to_lowercase())
                        .unwrap_or_default();
                    let pressed = matches!(
                        attempt_candidates
                            .iter()
                            .find(|c| c["id"] == choice.choice.as_str())
                            .and_then(|c| c["op"].as_str()),
                        Some("click" | "check")
                    );
                    if pressed
                        && !label.is_empty()
                        && subgoal.to_lowercase().contains(&format!("\"{label}\""))
                    {
                        return Ok(finish(
                            "done",
                            Some(format!("{target} was pressed as the subgoal asked; the page shows no visible change")),
                            &record,
                            &steps,
                            model_ms,
                        ));
                    }
                    // The step ran and checked out, but the page shows no
                    // change: it may have done the job (a click that only
                    // records something). The planner looks; GoMini cannot tell.
                    return Ok(finish(
                        "needs_help",
                        Some(format!(
                            "the step ran ({target}) but the page did not visibly change; check whether that finished the subgoal before asking again"
                        )),
                        &record,
                        &steps,
                        model_ms,
                    ));
                }
                let applied = call(
                    caller,
                    "decision.apply",
                    json!({"tabId": tab, "choose": action.choose, "value": action.value}),
                )
                .await;
                let mut ok = applied.is_ok();
                // Only a step that ran counts for "no progress": one refused
                // because the page moved on is simply tried again.
                if ok {
                    last = Some((choice.choice.clone(), revision));
                }
                let stale = applied.as_ref().err().is_some_and(|fail| {
                    fail.code == ErrorCode::TargetStale || fail.message.starts_with("stale")
                });
                entry["ran"] = applied
                    .as_ref()
                    .map(|r| r["ran"].clone())
                    .unwrap_or(Value::Null);
                if let Err(fail) = &applied {
                    entry["error"] = json!(fail.message);
                    // Covered by another offered control (the "+" of a
                    // closed pie menu): press that one, once, and go on.
                    let cover = fail.message.split_once(" is covered by ").map(|(_, rest)| {
                        rest.split("; dismiss").next().unwrap_or(rest).to_string()
                    });
                    let opener = cover.as_deref().and_then(|cover| {
                        attempt_candidates
                            .iter()
                            .find(|c| c["op"] == "click" && c["target"].as_str() == Some(cover))
                            .and_then(|c| c["id"].as_str().map(str::to_string))
                    });
                    if let Some(opener) =
                        opener.filter(|id| !steps.iter().any(|s| s["choice"] == id.as_str()))
                    {
                        let opened = call(
                            caller,
                            "decision.apply",
                            json!({"tabId": tab, "choose": opener}),
                        )
                        .await;
                        steps.push(entry);
                        steps.push(json!({"choice": opener, "ok": opened.is_ok()}));
                        if opened.is_ok() {
                            steps_ran += 1;
                            recent.push(json!({"choice": opener, "ok": true}));
                            let _ = call(caller, "wait.settle", json!({"tabId": tab})).await;
                            continue;
                        }
                        return Ok(finish(
                            "needs_help",
                            Some(fail.message.clone()),
                            &record,
                            &steps,
                            model_ms,
                        ));
                    }
                    // The browser's own gates (approval, sign-in, a human
                    // check) are the planner's to handle, not GoMini's.
                    if matches!(
                        fail.code,
                        ErrorCode::ApprovalRequired
                            | ErrorCode::HumanHandoff
                            | ErrorCode::PolicyDenied
                            | ErrorCode::TargetBlocked
                    ) {
                        steps.push(entry);
                        return Ok(finish(
                            "needs_help",
                            Some(fail.message.clone()),
                            &record,
                            &steps,
                            model_ms,
                        ));
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
                        ok = holds(&tab, reference, expected).await == Ok(true)
                            || attempt_candidates
                                .iter()
                                .any(|c| c["id"] == action.choose.as_str() && c["op"] == "select");
                        entry["readback"] = json!(ok);
                        bound.retain(|(n, r)| n != name && *r != reference);
                        bound.push((name.to_string(), reference));
                    }
                }
                entry["ok"] = json!(ok);
                if ok {
                    steps_ran += 1;
                }
                if stale {
                    entry["retried"] = json!(true);
                    steps.push(entry);
                    let _ = call(caller, "wait.settle", json!({"tabId": tab})).await;
                    continue;
                }
                recent.push(json!({"choice": choice.choice, "ok": ok}));
                if !ok && steps.iter().rev().take(1).any(|s| s["ok"] == false) {
                    steps.push(entry);
                    record = call(caller, "decision.record", record_params.clone())
                        .await
                        .unwrap_or(record);
                    return Ok(finish(
                        "needs_help",
                        Some("two steps in a row failed".into()),
                        &record,
                        &steps,
                        model_ms,
                    ));
                }
            }
        }
        steps.push(entry);
    }
    // The evidence shows the page as the last step left it.
    record = call(caller, "decision.record", record_params.clone())
        .await
        .unwrap_or(record);
    Ok(finish(
        "needs_help",
        Some(format!("the step budget ({max_steps}) is used")),
        &record,
        &steps,
        model_ms,
    ))
}

/// The checkboxes a subgoal names in quotes, as (candidate id, label,
/// checked), when every quoted name is a checkbox on the page and the
/// subgoal ticks rather than clears. Empty otherwise.
fn named_boxes(subgoal: &str, record: &Value) -> Vec<(String, String, bool)> {
    if decision::clears(subgoal) {
        return Vec::new();
    }
    let quoted: Vec<String> = subgoal
        .split('"')
        .skip(1)
        .step_by(2)
        .map(|name| name.trim().to_lowercase())
        .filter(|name| !name.is_empty())
        .collect();
    let boxes: Vec<(String, String, bool)> = record["candidates"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| {
            let rest = c["target"].as_str()?.strip_prefix("checkbox \"")?;
            let (label, after) = rest.split_once('"')?;
            let state = after.split(" in \"").next().unwrap_or(after);
            Some((
                c["id"].as_str()?.to_string(),
                label.trim().to_lowercase(),
                state.contains(" checked"),
            ))
        })
        .collect();
    let named: Vec<(String, String, bool)> = boxes
        .into_iter()
        .filter(|(_, label, _)| quoted.contains(label))
        .collect();
    if quoted.is_empty()
        || !quoted
            .iter()
            .all(|q| named.iter().any(|(_, label, _)| label == q))
    {
        return Vec::new();
    }
    named
}

/// Final steps a subgoal may end with that GoMini leaves to the planner.
const FINAL_WORDS: [&str; 12] = [
    "send", "submit", "search", "save", "post", "confirm", "apply", "book", "forward", "reply",
    "log in", "sign in",
];

/// The control for the final step a subgoal names ("... and send it") when
/// no step pressed it: `[106] clickable "(icon: send reply)"`.
fn final_step(subgoal: &str, record: &Value, steps: &[Value]) -> Option<String> {
    let lower = subgoal.to_lowercase();
    // Only a last clause that starts with the step: "... and send it",
    // "..., then click Submit".
    let (_, tail) = lower
        .rsplit_once(" and ")
        .or_else(|| lower.rsplit_once(", then "))?;
    let tail = ["then ", "click ", "press ", "the "]
        .iter()
        .fold(tail.trim(), |t, lead| t.strip_prefix(lead).unwrap_or(t));
    let word = FINAL_WORDS.iter().find(|word| tail.starts_with(*word))?;
    let candidate = record["candidates"].as_array()?.iter().find(|c| {
        c["ref"].is_u64()
            && c["op"] == "click"
            && c["target"].as_str().is_some_and(|t| {
                let t = t.to_lowercase();
                t.contains(word)
                    && !["close", "cancel", "discard"]
                        .iter()
                        .any(|no| t.contains(no))
            })
    })?;
    let id = candidate["id"].as_str()?;
    if steps.iter().any(|s| s["choice"] == id && s["held"] != true) {
        return None;
    }
    Some(format!(
        "[{}] {}",
        candidate["ref"],
        candidate["target"].as_str()?
    ))
}

/// Whether the field `reference` holds `expected` (in a picker's own format
/// too). `Err(true)` when the field is gone from the page.
async fn holds(tab: &str, reference: u64, expected: &str) -> Result<bool, bool> {
    match page::agent(tab, "holds", json!([reference, expected]), 4_000).await {
        Ok(out) => match out["holds"].as_bool() {
            Some(held) => Ok(held),
            None => Err(out["error"] == "stale"),
        },
        Err(_) => Err(false),
    }
}

/// The supplied values that are not (or no longer) in the field GoMini typed
/// them into, by name.
async fn unfilled(tab: &str, params: &Value, bound: &[(String, u64)]) -> Vec<String> {
    let mut missing = Vec::new();
    for (name, expected) in params["values"].as_object().into_iter().flatten() {
        let Some(expected) = expected.as_str() else {
            continue;
        };
        let filled = match bound.iter().find(|(n, _)| n == name) {
            Some((_, reference)) => match holds(tab, *reference, expected).await {
                Ok(held) => held,
                // The field is gone because the page moved on (the search
                // ran, the form was sent): the value was used.
                Err(stale) => stale,
            },
            None => false,
        };
        if !filled {
            missing.push(name.clone());
        }
    }
    missing
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
        "gomini.delegate" => {
            let mut out = delegate(caller, params).await?;
            // What the steps changed since the planner last looked, so a
            // finished delegation needs no read to check.
            if out["status"] == "done" {
                if let Ok(changes) =
                    call(caller, "state.diff", json!({"tabId": params.get("tabId")})).await
                {
                    for key in ["changes", "page", "navigated"] {
                        if let Some(value) = changes.get(key).filter(|v| !v.is_null()) {
                            out[key] = value.clone();
                        }
                    }
                }
            }
            Ok(out)
        }
        _ => Err(Fail::new(ErrorCode::MethodNotFound, method.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_final_step_left_undone_is_named_with_its_reference() {
        let record = json!({"candidates": [
            {"id": "r105", "op": "click", "ref": 105, "target": "clickable \"(icon: close reply)\""},
            {"id": "r106", "op": "click", "ref": 106, "target": "clickable \"(icon: send reply)\""},
            {"id": "done", "op": "done", "target": "the completion condition holds"},
        ]});
        assert_eq!(
            final_step("Open the email, type the reply and send it", &record, &[]).as_deref(),
            Some("[106] clickable \"(icon: send reply)\"")
        );
        assert_eq!(final_step("Type the reply", &record, &[]), None);
        assert_eq!(
            final_step(
                "Type the reply and send it",
                &record,
                &[json!({"choice": "r106"})]
            ),
            None
        );
    }

    #[test]
    fn named_boxes_are_the_quoted_checkboxes() {
        let record = json!({"candidates": [
            {"id": "r1", "target": "checkbox \"JjH\""},
            {"id": "r3", "target": "checkbox \"Hhc\" checked"},
            {"id": "r4", "target": "checkbox \"4jUYvW\""},
            {"id": "r7", "target": "button \"Submit\""},
        ]});
        let named = named_boxes(
            "Check the boxes \"Hhc\" and \"4jUYvW\" and nothing else",
            &record,
        );
        assert_eq!(
            named,
            vec![
                ("r3".to_string(), "hhc".to_string(), true),
                ("r4".to_string(), "4juyvw".to_string(), false)
            ]
        );
        // A quoted name that is not a box, or a subgoal that clears: no rule.
        assert!(named_boxes("Check \"Hhc\" and press \"Go\"", &record).is_empty());
        assert!(named_boxes("Uncheck \"Hhc\"", &record).is_empty());
    }

    #[test]
    fn origins_compare_scheme_and_host() {
        assert_eq!(origin("https://Shop.example/a/b?c"), "https://shop.example");
        assert_eq!(origin("http://127.0.0.1:5000"), "http://127.0.0.1:5000");
        assert_ne!(
            origin("https://shop.example/x"),
            origin("https://evil.example/x")
        );
    }
}
