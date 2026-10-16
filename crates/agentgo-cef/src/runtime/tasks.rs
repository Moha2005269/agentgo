//! The agent task engine: the calling model plans, AgentGo stores and runs
//! the plan (`agent.*`, `batch.*`, `recovery.*`).
//!
//! A task is a goal plus typed steps (`{method, params}`), validated against
//! the registry when it is made. Steps run through the normal dispatcher, so
//! leases, approvals, handoffs and the operator's pause/stop gate apply to
//! every step exactly as to a direct call. Tasks are kept as JSON in the
//! profile (`tasks.json`), so an interrupted task can be resumed after a
//! crash or restart.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;

use agentgo_protocol::ErrorCode;
use serde_json::{json, Map, Value};

use super::sessions::{self, Caller, Fail, Reply};
use super::{dispatch, exec, state};

/// Commands of the engine that run steps; they may not be steps themselves.
const RUNNERS: [&str; 10] = [
    "agent.run",
    "agent.act",
    "agent.retry",
    "agent.queue",
    "agent.subtask",
    "agent.parallel_tabs",
    "agent.parallel_profiles",
    "agent.resume_after_crash",
    "workflow.batch",
    "batch.commit",
];

thread_local! {
    static TASKS: RefCell<Option<BTreeMap<String, Value>>> = const { RefCell::new(None) };
    static ATTEMPTS: RefCell<BTreeMap<String, u64>> = const { RefCell::new(BTreeMap::new()) };
}

fn dir() -> PathBuf {
    state::data_dir().join("tasks")
}

fn file() -> PathBuf {
    dir().join(format!("{}.json", state::state().instance_id))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Each running instance keeps its tasks in `tasks/<instance>.json` and holds
/// `<instance>.lock`. Files of instances that are gone (their lock can be
/// taken) are adopted: their running tasks become `interrupted`.
fn load() -> BTreeMap<String, Value> {
    static LOCK: std::sync::OnceLock<Option<std::fs::File>> = std::sync::OnceLock::new();
    let _ = std::fs::create_dir_all(dir());
    LOCK.get_or_init(|| state::lock_exclusive(&file().with_extension("lock")));
    let mut tasks = BTreeMap::new();
    for entry in std::fs::read_dir(dir()).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "json") || path == file() {
            continue;
        }
        let Some(lock) = state::lock_exclusive(&path.with_extension("lock")) else {
            continue; // a live instance
        };
        let adopted: BTreeMap<String, Value> = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        tasks.extend(adopted);
        let _ = std::fs::remove_file(&path);
        drop(lock);
        let _ = std::fs::remove_file(path.with_extension("lock"));
    }
    for task in tasks.values_mut() {
        if task["status"] == "running" {
            task["status"] = json!("interrupted");
        }
    }
    tasks
}

fn with<R>(f: impl FnOnce(&mut BTreeMap<String, Value>) -> R) -> R {
    TASKS.with(|slot| f(slot.borrow_mut().get_or_insert_with(load)))
}

fn save() {
    let text = with(|tasks| {
        // ponytail: whole file rewritten per change; fine for hundreds of tasks.
        while tasks.len() > 300 {
            let oldest = tasks
                .iter()
                .filter(|(_, t)| t["status"] != "running")
                .min_by_key(|(_, t)| t["created"].as_u64().unwrap_or(0))
                .map(|(id, _)| id.clone());
            match oldest {
                Some(id) => tasks.remove(&id),
                None => break,
            };
        }
        serde_json::to_vec_pretty(tasks).unwrap_or_default()
    });
    let _ = std::fs::write(file(), text);
}

fn id_of(params: &Value) -> Option<String> {
    ["taskId", "batchId", "id"]
        .iter()
        .find_map(|key| params[*key].as_str().map(str::to_string))
}

/// The task a caller names; agents see their own tasks and interrupted ones.
fn get(caller: &Caller, params: &Value) -> Result<Value, Fail> {
    let id = id_of(params).ok_or_else(|| Fail::params("taskId is required"))?;
    let task = with(|tasks| tasks.get(&id).cloned())
        .ok_or_else(|| Fail::params(format!("unknown task {id}; agent.monitor lists tasks")))?;
    if caller.is_shell()
        || task["session"] == caller.session.as_str()
        || task["status"] == "interrupted"
    {
        Ok(task)
    } else {
        Err(Fail::new(
            ErrorCode::Unauthorized,
            format!("task {id} belongs to another agent session"),
        ))
    }
}

fn put(task: Value) {
    let id = task["id"].as_str().unwrap_or_default().to_string();
    with(|tasks| tasks.insert(id, task));
    save();
}

fn describe(method: &str) -> Option<String> {
    agentgo_shell_bridge::ShellMethodRegistry::new()
        .lookup(method)
        .filter(|m| m.status == agentgo_shell_bridge::MethodStatus::Implemented)
        .map(|m| m.description.to_string())
        .or_else(|| agentgo_shell_bridge::extra::find(method).map(|e| e.1.to_string()))
}

/// Checks steps against the registry and marks each read-only or acting.
pub(super) fn validate(steps: &Value) -> Result<Vec<Value>, Fail> {
    let list = steps
        .as_array()
        .filter(|list| list.len() <= 200)
        .ok_or_else(|| Fail::params("steps must be a list of at most 200 {method, params}"))?;
    let mut out = Vec::new();
    for (index, step) in list.iter().enumerate() {
        let method = step["method"].as_str().unwrap_or_default();
        if RUNNERS.contains(&method) {
            return Err(Fail::params(format!(
                "step {index}: {method} runs steps itself and cannot be a step"
            )));
        }
        if let Some(reason) = agentgo_shell_bridge::extra::unsupported(method) {
            return Err(Fail::new(
                ErrorCode::CapabilityUnsupported,
                format!("step {index}: {method}: {reason}"),
            ));
        }
        if describe(method).is_none() {
            return Err(Fail::new(
                ErrorCode::MethodNotFound,
                format!("step {index}: {method:?} is not a working command; browser.capabilities lists them"),
            ));
        }
        let read_only = dispatch::is_read_only(method);
        out.push(json!({
            "method": method,
            "params": step.get("params").cloned().unwrap_or_else(|| json!({})),
            "readOnly": read_only,
            "risk": risk(method, read_only),
        }));
    }
    Ok(out)
}

fn risk(method: &str, read_only: bool) -> &'static str {
    if read_only {
        "read"
    } else if method.starts_with("form.submit")
        || method.contains("delete")
        || method.contains("clear")
        || method.starts_with("credentials.")
        || method.starts_with("native.")
        || method.starts_with("download.")
    {
        "sensitive"
    } else {
        "act"
    }
}

fn new_task(caller: &Caller, params: &Value, steps: Vec<Value>) -> Value {
    let id = format!("task-{:x}-{}", now_ms(), with(|t| t.len()));
    let attempts = ATTEMPTS
        .with(|a| a.borrow().get(&caller.session).copied())
        .unwrap_or(1);
    json!({
        "id": id,
        "session": caller.session,
        "goal": params["goal"].as_str().unwrap_or(""),
        "steps": steps,
        "cursor": 0,
        "results": [],
        "status": "planned",
        "priority": params["priority"].as_i64().unwrap_or(0),
        "budget": {
            "maxSteps": params.pointer("/budget/maxSteps").and_then(Value::as_u64),
            "maxMs": params.pointer("/budget/maxMs").and_then(Value::as_u64),
        },
        "used": {"steps": 0, "ms": 0},
        "maxAttempts": params["maxAttempts"].as_u64().unwrap_or(attempts).clamp(1, 10),
        "parent": params["parent"],
        "checkpoints": {},
        "tabId": params["tabId"],
        "created": now_ms(),
    })
}

/// Summary without the bulky parts, for listings.
fn brief(task: &Value) -> Value {
    json!({
        "taskId": task["id"],
        "goal": task["goal"],
        "status": task["status"],
        "cursor": task["cursor"],
        "steps": task["steps"].as_array().map_or(0, Vec::len),
        "priority": task["priority"],
        "parent": task["parent"],
        "error": task["error"],
        "note": task["note"],
    })
}

fn view(task: &Value) -> Value {
    let mut out = brief(task);
    out["plan"] = task["steps"].clone();
    out["results"] = task["results"].clone();
    out["budget"] = task["budget"].clone();
    out["used"] = task["used"].clone();
    out["maxAttempts"] = task["maxAttempts"].clone();
    out
}

fn retryable(code: ErrorCode) -> bool {
    matches!(
        code,
        ErrorCode::Timeout | ErrorCode::TargetStale | ErrorCode::Internal
    )
}

/// Tabs the caller drives, with their addresses.
fn own_tabs(caller: &Caller) -> Map<String, Value> {
    state::content_tab_ids()
        .into_iter()
        .filter(|tab| {
            if caller.is_shell() {
                *tab == state::active_tab_id()
            } else {
                sessions::lease_holder(tab).as_deref() == Some(caller.session.as_str())
            }
        })
        .map(|tab| {
            let url = state::tab_url(&tab);
            (tab, json!(url))
        })
        .collect()
}

/// Runs steps from the cursor until done, a failure, the budget or `limit`.
async fn execute(caller: &Caller, mut task: Value, limit: Option<usize>) -> Value {
    let started = std::time::Instant::now();
    let steps = task["steps"].as_array().cloned().unwrap_or_default();
    let attempts = task["maxAttempts"].as_u64().unwrap_or(1).max(1);
    task["status"] = json!("running");
    task["error"] = Value::Null;
    put(task.clone());
    let mut ran = 0usize;
    loop {
        let cursor = task["cursor"].as_u64().unwrap_or(0) as usize;
        if cursor >= steps.len() {
            task["status"] = json!("done");
            break;
        }
        if limit.is_some_and(|limit| ran >= limit) {
            task["status"] = json!("paused");
            break;
        }
        let used_steps = task["used"]["steps"].as_u64().unwrap_or(0);
        if task["budget"]["maxSteps"]
            .as_u64()
            .is_some_and(|max| used_steps >= max)
            || task["budget"]["maxMs"]
                .as_u64()
                .is_some_and(|max| started.elapsed().as_millis() as u64 >= max)
        {
            task["status"] = json!("budget");
            task["error"] = json!({"code": "recovery.exhausted", "message": "the task's budget (agent.budget) is used up"});
            break;
        }
        // Stopped by the operator or replanned meanwhile.
        if let Some(current) = with(|t| t.get(task["id"].as_str().unwrap_or("")).cloned()) {
            if current["status"] == "cancelled" {
                task = current;
                break;
            }
        }
        let step = &steps[cursor];
        let method = step["method"].as_str().unwrap_or_default().to_string();
        let mut params = step["params"].clone();
        if let (Some(tab), Some(map)) = (task["tabId"].as_str(), params.as_object_mut()) {
            map.entry("tabId").or_insert_with(|| json!(tab));
        }
        let mut tries = 0;
        let reply = loop {
            tries += 1;
            let reply = match resolve(caller, &mut params).await {
                Ok(()) => Box::pin(dispatch::run(caller, &method, params.clone())).await,
                Err(fail) => Err(fail),
            };
            match &reply {
                Err(fail) if retryable(fail.code) && tries < attempts => exec::sleep(300).await,
                _ => break reply,
            }
        };
        task["used"]["steps"] = json!(used_steps + 1);
        ran += 1;
        let results = task["results"].as_array_mut().expect("results");
        results.truncate(cursor);
        match reply {
            Ok(value) => {
                results.push(json!({"step": cursor, "method": method, "ok": true, "tries": tries, "result": value}));
                task["cursor"] = json!(cursor + 1);
            }
            Err(fail) => {
                let error = json!({"code": fail.code.as_str(), "message": fail.message, "data": fail.data, "step": cursor});
                results.push(json!({"step": cursor, "method": method, "ok": false, "tries": tries, "error": error}));
                task["error"] = error;
                task["status"] = json!("failed");
                break;
            }
        }
        put(task.clone());
    }
    let ms = task["used"]["ms"].as_u64().unwrap_or(0) + started.elapsed().as_millis() as u64;
    task["used"]["ms"] = json!(ms);
    put(task.clone());
    view(&task)
}

/// A step's `find: {by, ...}` names its element by a locator (`locate.<by>`)
/// that is resolved when the step runs, so a plan survives page changes.
pub(super) async fn resolve(caller: &Caller, params: &mut Value) -> Result<(), Fail> {
    let Some(find) = params.get("find").cloned() else {
        return Ok(());
    };
    let by = find["by"].as_str().unwrap_or("css");
    let mut query = find.clone();
    if let Some(tab) = params.get("tabId") {
        query["tabId"] = tab.clone();
    }
    let found = Box::pin(dispatch::run(caller, &format!("locate.{by}"), query)).await?;
    let matches = found["matches"].as_array().cloned().unwrap_or_default();
    let index = find["nth"].as_u64().unwrap_or(0) as usize;
    if matches.len() > 1 && find.get("nth").is_none() {
        return Err(Fail::new(
            ErrorCode::TargetAmbiguous,
            format!("find {find} matches {} elements; add nth", matches.len()),
        )
        .with(json!({"candidates": matches})));
    }
    let hit = matches.get(index).ok_or_else(|| {
        Fail::new(
            ErrorCode::TargetStale,
            format!("find {find} matches nothing on the page"),
        )
    })?;
    if let Some(map) = params.as_object_mut() {
        map.remove("find");
        map.insert("ref".into(), hit["ref"].clone());
    }
    Ok(())
}

async fn restore_tabs(caller: &Caller, tabs: &Value) -> Vec<Value> {
    let mut restored = Vec::new();
    for (tab, url) in tabs.as_object().cloned().unwrap_or_default() {
        let url = url.as_str().unwrap_or_default().to_string();
        if !state::tab_exists(&tab) {
            restored.push(json!({"tabId": tab, "restored": false, "reason": "tab closed"}));
            continue;
        }
        if state::tab_url(&tab) == url {
            restored.push(json!({"tabId": tab, "restored": true, "url": url}));
            continue;
        }
        let reply = Box::pin(dispatch::run(
            caller,
            "navigate.url",
            json!({"tabId": tab, "url": url}),
        ))
        .await;
        restored.push(json!({"tabId": tab, "restored": reply.is_ok(), "url": url}));
    }
    restored
}

fn compare(a: &Value, b: &Value) -> Value {
    let left = a.as_array().cloned().unwrap_or_default();
    let right = b.as_array().cloned().unwrap_or_default();
    let mut same = Vec::new();
    let mut different = Vec::new();
    for index in 0..left.len().max(right.len()) {
        let (l, r) = (left.get(index), right.get(index));
        let strip = |v: Option<&Value>| {
            v.map(|v| {
                v.get("result")
                    .or(v.get("error"))
                    .cloned()
                    .unwrap_or(v.clone())
            })
        };
        if strip(l) == strip(r) {
            same.push(index);
        } else {
            different.push(json!({"index": index, "a": strip(l), "b": strip(r)}));
        }
    }
    json!({"same": same, "different": different, "equal": different.is_empty()})
}

fn dedupe(items: Vec<Value>, keys: &[String]) -> (Vec<Value>, usize) {
    let mut seen = std::collections::HashSet::new();
    let total = items.len();
    let kept: Vec<Value> = items
        .into_iter()
        .filter(|item| {
            let key = if keys.is_empty() {
                item.to_string()
            } else {
                keys.iter()
                    .map(|k| item[k.as_str()].to_string())
                    .collect::<Vec<_>>()
                    .join("\u{1f}")
            };
            seen.insert(key)
        })
        .collect();
    let removed = total - kept.len();
    (kept, removed)
}

fn keys_of(params: &Value) -> Vec<String> {
    match (&params["key"], &params["keys"]) {
        (Value::String(key), _) => vec![key.clone()],
        (_, Value::Array(keys)) => keys
            .iter()
            .filter_map(|k| k.as_str().map(str::to_string))
            .collect(),
        _ => Vec::new(),
    }
}

/// What kind of failure an error is, and what to call next.
pub fn classify(code: &str, message: &str) -> Value {
    let lower = message.to_lowercase();
    let (class, retry, next) = match code {
        "target.stale" | "recovery.stale" => (
            "stale_reference",
            true,
            "observe.page, then use the new reference",
        ),
        "target.ambiguous" => (
            "ambiguous_target",
            false,
            "observe.page and pick one of the candidates by reference",
        ),
        "target.blocked" => (
            "blocked_target",
            true,
            "dialog.list or observe.page: something covers the element",
        ),
        "timeout" if lower.contains("load") || lower.contains("navigat") => {
            ("slow_page", true, "wait.load, then retry")
        }
        "timeout" => (
            "timeout",
            true,
            "wait.settle, then retry with a larger timeout",
        ),
        "approval.required" => (
            "needs_approval",
            false,
            "approval.wait with data.approvalId, then repeat with approvalId",
        ),
        "handoff.required" => (
            "needs_person",
            false,
            "handoff.wait until the person finished, then continue",
        ),
        "policy.denied" => (
            "not_allowed",
            false,
            "do not retry; the person's settings or policy forbid it",
        ),
        "unauthorized" => ("not_your_tab", false, "tabs.create your own tab"),
        "capability.unsupported" => (
            "unsupported",
            false,
            "browser.capabilities lists what works",
        ),
        "invalid.params" | "method.notFound" => {
            ("bad_call", false, "fix the parameters (browser.guide)")
        }
        "recovery.exhausted" => (
            "budget_used",
            false,
            "agent.budget to raise the budget, or stop",
        ),
        _ if lower.contains("crash") || lower.contains("renderer") => {
            ("renderer_crash", true, "tab.reload, then observe.page")
        }
        _ if lower.contains("net::") || lower.contains("network") => {
            ("network_failed", true, "network.failures, then tab.reload")
        }
        _ if lower.contains("popup") => (
            "popup_blocked",
            false,
            "recovery.popup_blocked, then tabs.create with the address",
        ),
        _ if lower.contains("permission") => {
            ("permission_denied", false, "recovery.permission_denied")
        }
        _ => (
            "unknown",
            code == "internal",
            "observe.page to see the state, then decide",
        ),
    };
    json!({"code": code, "class": class, "retryable": retry, "next": next})
}

/// Recent runtime events whose line contains `marker`.
fn events(marker: &str) -> Vec<String> {
    state::state()
        .events
        .lock()
        .map(|events| {
            events
                .iter()
                .rev()
                .filter(|line| line.contains(marker))
                .take(30)
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

fn quoted(line: &str) -> Option<String> {
    let start = line.find('"')?;
    let end = line[start + 1..].find('"')?;
    Some(line[start + 1..start + 1 + end].to_string())
}

/// The session that owns a task.
pub fn owner(id: &str) -> Option<String> {
    with(|tasks| {
        tasks
            .get(id)
            .and_then(|t| t["session"].as_str().map(str::to_string))
    })
}

/// The person asked the agent to plan the rest again (`shell.agent_replan`).
pub fn request_replan(id: &str, note: &str) {
    let changed = with(|tasks| {
        tasks.get_mut(id).map(|task| {
            if task["status"] != "running" {
                task["status"] = json!("replan");
            }
            task["note"] = json!(note);
        })
    });
    if changed.is_some() {
        save();
    }
}

/// Every engine command, routed from the dispatcher.
pub fn handles(method: &str) -> bool {
    matches!(
        method,
        "agent.plan"
            | "agent.preview"
            | "agent.dry_run"
            | "agent.run"
            | "agent.act"
            | "agent.monitor"
            | "agent.checkpoint"
            | "agent.restore"
            | "agent.retry"
            | "agent.replan"
            | "agent.explain"
            | "agent.compare"
            | "agent.budget"
            | "agent.parallel_tabs"
            | "agent.parallel_profiles"
            | "agent.queue"
            | "agent.priority"
            | "agent.lock"
            | "agent.unlock"
            | "agent.subtask"
            | "agent.merge_results"
            | "agent.deduplicate_results"
            | "agent.resume_after_crash"
            | "batch.begin"
            | "batch.add"
            | "batch.preview"
            | "batch.transaction_status"
            | "batch.rollback_ui"
            | "batch.dedupe"
            | "batch.idempotency_key"
            | "recovery.classify_error"
            | "recovery.popup_blocked"
            | "recovery.permission_denied"
            | "recovery.false_completion"
            | "recovery.max_attempts"
    )
}

pub async fn command(caller: &Caller, method: &str, params: &Value) -> Reply {
    match method {
        "agent.plan" | "batch.begin" => {
            let steps = validate(params.get("steps").unwrap_or(&json!([])))?;
            let mut task = new_task(caller, params, steps);
            if method == "batch.begin" {
                task["goal"] = json!(params["name"]
                    .as_str()
                    .or(params["goal"].as_str())
                    .unwrap_or("batch"));
                task["checkpoints"]["begin"] = json!({"cursor": 0, "tabs": own_tabs(caller)});
            }
            put(task.clone());
            let mut out = view(&task);
            out["batchId"] = task["id"].clone();
            Ok(out)
        }
        "agent.preview" | "agent.dry_run" | "batch.preview" => {
            let steps = match params.get("steps") {
                Some(steps) => validate(steps)?,
                None => get(caller, params)?["steps"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default(),
            };
            let acting = steps.iter().filter(|s| s["readOnly"] == false).count();
            let plan: Vec<Value> = steps
                .iter()
                .enumerate()
                .map(|(index, step)| {
                    let mut step = step.clone();
                    step["index"] = json!(index);
                    step["description"] = json!(describe(step["method"].as_str().unwrap_or("")));
                    step
                })
                .collect();
            Ok(json!({"plan": plan, "steps": plan.len(), "acting": acting, "executed": false}))
        }
        "agent.run" | "agent.retry" | "agent.act" => {
            let mut task = if method == "agent.run" && id_of(params).is_none() {
                let steps = validate(
                    params
                        .get("steps")
                        .ok_or_else(|| Fail::params("agent.run needs taskId or steps"))?,
                )?;
                new_task(caller, params, steps)
            } else {
                get(caller, params)?
            };
            if task["status"] == "running" {
                return Err(Fail::params(
                    "the task is already running; agent.monitor shows its progress",
                ));
            }
            if task["status"] == "interrupted" {
                task["session"] = json!(caller.session);
            }
            // A retry may add parameters to the failed step, such as approvalId.
            if let (Some(extra), Some(cursor)) =
                (params["params"].as_object(), task["cursor"].as_u64())
            {
                if let Some(step) = task["steps"].get_mut(cursor as usize) {
                    if let Some(map) = step["params"].as_object_mut() {
                        map.extend(extra.clone());
                    }
                }
            }
            if let Some(attempts) = params["maxAttempts"].as_u64() {
                task["maxAttempts"] = json!(attempts.clamp(1, 10));
            }
            Ok(execute(caller, task, (method == "agent.act").then_some(1)).await)
        }
        "agent.monitor" | "batch.transaction_status" => match id_of(params) {
            Some(_) => Ok(view(&get(caller, params)?)),
            None => Ok(json!({"tasks": with(|tasks| tasks
                .values()
                .filter(|t| caller.is_shell() || t["session"] == caller.session.as_str() || t["status"] == "interrupted")
                .map(brief)
                .collect::<Vec<_>>())})),
        },
        "agent.checkpoint" => {
            let mut task = get(caller, params)?;
            let name = params["name"].as_str().unwrap_or("last").to_string();
            let tabs = own_tabs(caller);
            task["checkpoints"][&name] =
                json!({"cursor": task["cursor"], "tabs": tabs, "at": now_ms()});
            put(task.clone());
            Ok(
                json!({"taskId": task["id"], "checkpoint": name, "cursor": task["cursor"], "tabs": tabs}),
            )
        }
        "agent.restore" | "batch.rollback_ui" => {
            let mut task = get(caller, params)?;
            let default = if method == "batch.rollback_ui" {
                "begin"
            } else {
                "last"
            };
            let name = params["name"].as_str().unwrap_or(default);
            let point = task["checkpoints"][name].clone();
            if point.is_null() {
                return Err(Fail::params(format!("task has no checkpoint {name:?}")));
            }
            let restored = restore_tabs(caller, &point["tabs"]).await;
            if method == "agent.restore" {
                let cursor = point["cursor"].as_u64().unwrap_or(0);
                task["cursor"] = json!(cursor);
                if let Some(results) = task["results"].as_array_mut() {
                    results.truncate(cursor as usize);
                }
                task["status"] = json!("planned");
                put(task.clone());
            }
            Ok(
                json!({"taskId": task["id"], "checkpoint": name, "cursor": task["cursor"], "tabs": restored}),
            )
        }
        "agent.replan" => {
            let mut task = get(caller, params)?;
            let steps = validate(
                params
                    .get("steps")
                    .ok_or_else(|| Fail::params("steps are required"))?,
            )?;
            let cursor = task["cursor"].as_u64().unwrap_or(0) as usize;
            let list = task["steps"].as_array_mut().expect("steps");
            list.truncate(cursor);
            list.extend(steps);
            if task["status"] != "running" && task["status"] != "interrupted" {
                task["status"] = json!("planned");
            }
            task["error"] = Value::Null;
            put(task.clone());
            Ok(view(&task))
        }
        "batch.add" => {
            let mut task = get(caller, params)?;
            let steps = match params.get("steps") {
                Some(steps) => validate(steps)?,
                None => validate(
                    &json!([{"method": params["method"], "params": params.get("params").cloned().unwrap_or(json!({}))}]),
                )?,
            };
            task["steps"].as_array_mut().expect("steps").extend(steps);
            put(task.clone());
            Ok(
                json!({"batchId": task["id"], "steps": task["steps"].as_array().map_or(0, Vec::len)}),
            )
        }
        "batch.dedupe" => {
            let mut task = get(caller, params)?;
            let cursor = task["cursor"].as_u64().unwrap_or(0) as usize;
            let steps = task["steps"].as_array().cloned().unwrap_or_default();
            let (done, rest) = steps.split_at(cursor.min(steps.len()));
            let rest: Vec<Value> = rest.iter().map(|s| json!({"method": s["method"], "params": s["params"], "readOnly": s["readOnly"], "risk": s["risk"]})).collect();
            let (kept, removed) = dedupe(rest, &[]);
            task["steps"] = json!(done.iter().cloned().chain(kept).collect::<Vec<_>>());
            put(task.clone());
            Ok(
                json!({"batchId": task["id"], "removed": removed, "steps": task["steps"].as_array().map_or(0, Vec::len)}),
            )
        }
        "batch.idempotency_key" => {
            let steps = match params.get("steps") {
                Some(steps) => steps.clone(),
                None => get(caller, params)?["steps"].clone(),
            };
            let text = steps.to_string();
            let hash = text.bytes().fold(0xcbf29ce484222325u64, |h, b| {
                (h ^ b as u64).wrapping_mul(0x100000001b3)
            });
            Ok(
                json!({"idempotencyKey": format!("batch-{hash:016x}"), "use": "pass it as idempotencyKey to agent.run or workflow.batch; a repeated call returns the first reply"}),
            )
        }
        "agent.explain" => {
            let task = get(caller, params)?;
            let results = task["results"].as_array().cloned().unwrap_or_default();
            let steps: Vec<Value> = task["steps"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .iter()
                .enumerate()
                .map(|(index, step)| {
                    let method = step["method"].as_str().unwrap_or("");
                    let outcome = results.iter().find(|r| r["step"] == index);
                    json!({
                        "index": index,
                        "method": method,
                        "does": describe(method),
                        "risk": step["risk"],
                        "params": step["params"],
                        "state": match outcome { Some(r) if r["ok"] == true => "done", Some(_) => "failed", None => "pending" },
                        "error": outcome.map(|r| r["error"].clone()),
                    })
                })
                .collect();
            Ok(
                json!({"taskId": task["id"], "goal": task["goal"], "status": task["status"], "steps": steps}),
            )
        }
        "agent.compare" => {
            let side = |key: &str| -> Result<Value, Fail> {
                match &params[key] {
                    Value::String(id) => {
                        Ok(get(caller, &json!({"taskId": id}))?["results"].clone())
                    }
                    other => Ok(other.clone()),
                }
            };
            Ok(compare(&side("a")?, &side("b")?))
        }
        "agent.budget" => {
            let mut task = get(caller, params)?;
            for key in ["maxSteps", "maxMs"] {
                if let Some(value) = params[key].as_u64() {
                    task["budget"][key] = json!(value);
                }
            }
            put(task.clone());
            Ok(json!({"taskId": task["id"], "budget": task["budget"], "used": task["used"]}))
        }
        "agent.priority" => {
            let mut task = get(caller, params)?;
            task["priority"] = json!(params["priority"]
                .as_i64()
                .ok_or_else(|| Fail::params("priority (a number) is required"))?);
            put(task.clone());
            Ok(brief(&task))
        }
        "agent.queue" => {
            if let Some(id) = id_of(params) {
                let mut task = get(caller, params)?;
                if task["status"] != "running" {
                    task["status"] = json!("queued");
                }
                put(task);
                if params["run"] != true {
                    return Ok(json!({"queued": id}));
                }
            }
            let mine = |t: &Value| caller.is_shell() || t["session"] == caller.session.as_str();
            if params["run"] != true {
                let mut queue: Vec<Value> = with(|tasks| {
                    tasks
                        .values()
                        .filter(|t| t["status"] == "queued" && mine(t))
                        .map(brief)
                        .collect()
                });
                queue.sort_by_key(|t| -t["priority"].as_i64().unwrap_or(0));
                return Ok(json!({"queue": queue}));
            }
            // Highest priority first, oldest first among equals.
            let mut finished = Vec::new();
            loop {
                let next = with(|tasks| {
                    tasks
                        .values()
                        .filter(|t| t["status"] == "queued" && mine(t))
                        .max_by_key(|t| {
                            (
                                t["priority"].as_i64().unwrap_or(0),
                                -(t["created"].as_i64().unwrap_or(0)),
                            )
                        })
                        .cloned()
                });
                let Some(task) = next else { break };
                finished.push(brief_of(execute(caller, task, None).await));
            }
            Ok(json!({"ran": finished}))
        }
        "agent.subtask" => {
            let parent = get(caller, params)?;
            let steps = validate(
                params
                    .get("steps")
                    .ok_or_else(|| Fail::params("steps are required"))?,
            )?;
            let mut child_params = params.clone();
            child_params["parent"] = parent["id"].clone();
            child_params["tabId"] = params
                .get("tabId")
                .cloned()
                .unwrap_or_else(|| parent["tabId"].clone());
            let child = new_task(caller, &child_params, steps);
            put(child.clone());
            if params["run"] == false {
                return Ok(view(&child));
            }
            Ok(execute(caller, child, None).await)
        }
        "agent.parallel_tabs" | "agent.parallel_profiles" => {
            let lanes = params["lanes"]
                .as_array()
                .filter(|lanes| !lanes.is_empty() && lanes.len() <= 8)
                .ok_or_else(|| Fail::params("lanes: 1..8 of {tabId | url, steps}"))?
                .clone();
            let mut tasks = Vec::new();
            let stamp = now_ms() & 0xffffff;
            for (index, lane) in lanes.iter().enumerate() {
                let steps = validate(&lane["steps"])?;
                // Each lane of parallel_profiles gets its own cookies and storage.
                let isolated = method == "agent.parallel_profiles";
                let tab = match lane["tabId"].as_str().filter(|_| !isolated) {
                    Some(tab) => tab.to_string(),
                    None => {
                        let mut create =
                            json!({"url": lane["url"].as_str().unwrap_or("about:blank")});
                        match lane["profile"].as_str() {
                            Some(profile) => create["profile"] = json!(profile),
                            None if isolated => {
                                create["profile"] = json!(format!("lane-{stamp:x}-{index}"));
                                create["ephemeral"] = json!(true);
                            }
                            None => {}
                        }
                        let created =
                            Box::pin(dispatch::run(caller, "tabs.create", create)).await?;
                        let tab = created["tabId"].as_str().unwrap_or_default().to_string();
                        if lane["url"].is_string() {
                            // Steps such as cookie writes need the lane's page committed.
                            let _ = Box::pin(dispatch::run(
                                caller,
                                "wait.load",
                                json!({"tabId": tab, "timeout": 15000}),
                            ))
                            .await;
                        }
                        tab
                    }
                };
                sessions::authorize_tab(caller, &tab)?;
                let mut lane_params = lane.clone();
                lane_params["tabId"] = json!(tab);
                lane_params["goal"] = json!(lane["goal"].as_str().unwrap_or("parallel lane"));
                let task = new_task(caller, &lane_params, steps);
                put(task.clone());
                tasks.push(task);
            }
            let slots: Vec<exec::Slot<Value>> = tasks
                .into_iter()
                .map(|task| {
                    let slot = exec::Slot::default();
                    let (out, caller) = (slot.clone(), caller.clone());
                    exec::spawn(async move { out.fill(execute(&caller, task, None).await) });
                    slot
                })
                .collect();
            let timeout = params["timeout"]
                .as_u64()
                .unwrap_or(300_000)
                .clamp(1_000, 1_800_000);
            let mut out = Vec::new();
            for slot in slots {
                out.push(
                    slot.wait(timeout)
                        .await
                        .unwrap_or_else(|| json!({"status": "timeout"})),
                );
            }
            let done = out.iter().filter(|r| r["status"] == "done").count();
            Ok(json!({"lanes": out, "done": done, "failed": out.len() - done}))
        }
        "agent.lock" | "agent.unlock" => {
            let tab = super::page::tab_of(caller, params)?;
            sessions::set_lock(caller, &tab, method == "agent.lock")?;
            Ok(json!({"tabId": tab, "locked": method == "agent.lock"}))
        }
        "agent.merge_results" | "agent.deduplicate_results" => {
            let mut items = Vec::new();
            if let Some(lists) = params["lists"].as_array() {
                for list in lists {
                    items.extend(
                        list.as_array()
                            .cloned()
                            .unwrap_or_else(|| vec![list.clone()]),
                    );
                }
            }
            if let Some(list) = params["items"].as_array() {
                items.extend(list.iter().cloned());
            }
            for id in params["taskIds"].as_array().cloned().unwrap_or_default() {
                let task = get(caller, &json!({"taskId": id}))?;
                for result in task["results"].as_array().cloned().unwrap_or_default() {
                    match &result["result"] {
                        Value::Array(list) => items.extend(list.iter().cloned()),
                        other => items.extend(
                            other
                                .get("items")
                                .and_then(Value::as_array)
                                .cloned()
                                .unwrap_or_else(|| vec![other.clone()]),
                        ),
                    }
                }
            }
            let keys = keys_of(params);
            let dedupe_now = method == "agent.deduplicate_results" || params["dedupe"] == true;
            let total = items.len();
            let (items, removed) = if dedupe_now {
                dedupe(items, &keys)
            } else {
                (items, 0)
            };
            Ok(json!({"items": items, "count": total - removed, "removed": removed}))
        }
        "agent.resume_after_crash" => match id_of(params) {
            Some(_) => {
                let mut task = get(caller, params)?;
                if task["status"] != "interrupted" && task["status"] != "failed" {
                    return Err(Fail::params(
                        "only an interrupted or failed task can be resumed",
                    ));
                }
                task["session"] = json!(caller.session);
                Ok(execute(caller, task, None).await)
            }
            None => Ok(
                json!({"interrupted": with(|tasks| tasks.values().filter(|t| t["status"] == "interrupted").map(brief).collect::<Vec<_>>())}),
            ),
        },
        "recovery.classify_error" => {
            let code = params["code"]
                .as_str()
                .ok_or_else(|| Fail::params("code is required"))?;
            Ok(classify(code, params["message"].as_str().unwrap_or("")))
        }
        "recovery.popup_blocked" => {
            let blocked: Vec<String> = events("popup-blocked")
                .iter()
                .filter_map(|line| quoted(line))
                .filter(|url| !url.is_empty())
                .collect();
            Ok(
                json!({"blocked": blocked, "why": "a page opened a window without a click; AgentGo blocks those", "next": "tabs.create with the address, or click the link that opens it"}),
            )
        }
        "recovery.permission_denied" => {
            let denied: Vec<Value> = events("permission-denied")
                .iter()
                .chain(events("permission-refused").iter())
                .map(|line| {
                    let origin = line
                        .split("origin=")
                        .nth(1)
                        .and_then(quoted)
                        .unwrap_or_default();
                    let kind = line
                        .split("kind=")
                        .nth(1)
                        .and_then(|s| s.split_whitespace().next())
                        .unwrap_or("")
                        .to_string();
                    json!({"origin": origin, "kind": kind})
                })
                .collect();
            Ok(
                json!({"denied": denied, "granted": super::extras::list()["sites"], "next": "only the person can grant a site permission (Settings, or permission.grant from the operator)"}),
            )
        }
        "recovery.false_completion" => {
            let mut check = json!({"timeout": params["timeout"].as_u64().unwrap_or(3_000)});
            for (from, to) in [
                ("text", "text"),
                ("selector", "selector"),
                ("url", "urlContains"),
                ("urlContains", "urlContains"),
                ("gone", "gone"),
            ] {
                if let Some(value) = params.get(from) {
                    check[to] = value.clone();
                }
            }
            if let Some(tab) = params.get("tabId") {
                check["tabId"] = tab.clone();
            }
            if check.as_object().is_some_and(|m| m.len() < 2) {
                return Err(Fail::params(
                    "name what completion looks like: text, selector, url or gone",
                ));
            }
            match Box::pin(dispatch::run(caller, "wait.condition", check)).await {
                Ok(_) => Ok(json!({"verified": true})),
                Err(fail) if fail.code == ErrorCode::Timeout => Ok(json!({
                    "verified": false,
                    "falseCompletion": true,
                    "next": "the action reported success but the expected result is not there: observe.page, then retry the step or replan",
                })),
                Err(fail) => Err(fail),
            }
        }
        "recovery.max_attempts" => {
            let attempts = params["attempts"]
                .as_u64()
                .ok_or_else(|| Fail::params("attempts (1..10) is required"))?
                .clamp(1, 10);
            if id_of(params).is_some() {
                let mut task = get(caller, params)?;
                task["maxAttempts"] = json!(attempts);
                put(task.clone());
                return Ok(json!({"taskId": task["id"], "maxAttempts": attempts}));
            }
            ATTEMPTS.with(|a| a.borrow_mut().insert(caller.session.clone(), attempts));
            Ok(json!({"maxAttempts": attempts, "scope": "new tasks of this session"}))
        }
        _ => Err(Fail::internal(format!("{method} is not a task command"))),
    }
}

fn brief_of(view: Value) -> Value {
    json!({"taskId": view["taskId"], "status": view["status"], "error": view["error"]})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_names_next_step() {
        assert_eq!(classify("target.stale", "")["class"], "stale_reference");
        assert_eq!(
            classify("timeout", "page did not load")["class"],
            "slow_page"
        );
        assert_eq!(
            classify("internal", "net::ERR_FAILED")["class"],
            "network_failed"
        );
        assert_eq!(classify("policy.denied", "")["retryable"], false);
    }

    #[test]
    fn dedupe_by_key_and_whole() {
        let items = vec![
            json!({"a": 1, "b": 1}),
            json!({"a": 1, "b": 2}),
            json!({"a": 1, "b": 1}),
        ];
        assert_eq!(dedupe(items.clone(), &[]).1, 1);
        assert_eq!(dedupe(items, &["a".into()]).1, 2);
    }

    #[test]
    fn compare_finds_differences() {
        let out = compare(
            &json!([{"result": 1}, {"result": 2}]),
            &json!([{"result": 1}, {"result": 3}]),
        );
        assert_eq!(out["same"], json!([0]));
        assert_eq!(out["different"][0]["index"], 1);
    }
}
