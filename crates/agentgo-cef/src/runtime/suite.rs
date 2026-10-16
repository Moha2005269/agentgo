//! Measurement, memory and housekeeping for agents:
//!
//! - **metrics** counted live from every command a session sends, plus task
//!   outcomes the planner reports (success, tokens, turns), and **benchmark**
//!   reports saved, compared and checked for regressions. This is what the
//!   A/B/C0/C1 experiment and Laya's training read.
//! - **memory**: notes an agent keeps between tasks, by scope.
//! - **research cache**: page data an agent already fetched, by address.
//! - **sessions**: named sets of open pages, saved and reopened; profile
//!   export, import and checkpoints.
//! - **domain policy**: sites agents may not use, or the only ones they may.
//! - **artifacts and files**: what agents saved (screenshots, PDFs, frames,
//!   recordings), listed, read, hashed, bundled and cleaned up; file
//!   operations stay inside AgentGo's own folders.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use agentgo_protocol::ErrorCode;
use serde_json::{json, Value};

use super::sessions::{Fail, Reply};
use super::state;

// ---------------------------------------------------------------------------
// metrics

#[derive(Default, Clone)]
struct Counters {
    commands: u64,
    actions: u64,
    errors: BTreeMap<String, u64>,
    recovered: u64,
    last_error: Option<String>,
    latencies: Vec<u64>,
    batches: Vec<u64>,
    uploads: (u64, u64),
    dialogs: (u64, u64),
    handoffs: Vec<u64>,
    reply_chars: u64,
    tasks: Vec<Value>,
}

thread_local! {
    static METRICS: RefCell<HashMap<String, Counters>> = RefCell::new(HashMap::new());
    static CACHE: RefCell<HashMap<String, (u64, Value)>> = RefCell::new(HashMap::new());
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Counts one finished command of a session.
pub fn observe(
    session: &str,
    method: &str,
    read_only: bool,
    ms: u64,
    params: &Value,
    reply: &Reply,
) {
    if method.starts_with("metrics.") || method.starts_with("benchmark.") {
        return;
    }
    METRICS.with(|all| {
        let mut all = all.borrow_mut();
        let c = all.entry(session.to_string()).or_default();
        c.commands += 1;
        if !read_only {
            c.actions += 1;
        }
        if c.latencies.len() < 5000 {
            c.latencies.push(ms);
        }
        let ok = reply.is_ok();
        match reply {
            Ok(value) => {
                c.reply_chars += value.to_string().len() as u64;
                if c.last_error.take().is_some_and(|failed| failed == method) {
                    c.recovered += 1;
                }
            }
            Err(fail) => {
                *c.errors.entry(fail.code.as_str().to_string()).or_default() += 1;
                c.last_error = Some(method.to_string());
            }
        }
        if method == "workflow.batch" || method == "batch" {
            c.batches
                .push(params["steps"].as_array().map_or(0, Vec::len) as u64);
        }
        if method == "act.upload" {
            if ok {
                c.uploads.0 += 1
            } else {
                c.uploads.1 += 1
            }
        }
        if method.starts_with("dialog.") && method != "dialog.list" {
            if ok {
                c.dialogs.0 += 1
            } else {
                c.dialogs.1 += 1
            }
        }
        if method == "handoff.wait" && ok {
            c.handoffs.push(ms);
        }
    });
}

fn counters(session: &str) -> Counters {
    METRICS.with(|all| all.borrow().get(session).cloned().unwrap_or_default())
}

fn ratio(part: u64, whole: u64) -> Value {
    if whole == 0 {
        Value::Null
    } else {
        json!((part as f64 / whole as f64 * 1000.0).round() / 1000.0)
    }
}

fn snapshot(c: &Counters) -> Value {
    let mut sorted = c.latencies.clone();
    sorted.sort_unstable();
    let pct = |p: f64| {
        sorted
            .get(((sorted.len() as f64 - 1.0) * p).round() as usize)
            .copied()
    };
    let reported = |key: &str| {
        c.tasks
            .iter()
            .filter_map(|task| task[key].as_f64())
            .collect::<Vec<_>>()
    };
    let succeeded = c.tasks.iter().filter(|task| task["ok"] == true).count() as u64;
    let tokens: f64 = reported("tokens").iter().sum();
    let turns: f64 = reported("turns").iter().sum();
    let accuracy = reported("accuracy");
    let errors: u64 = c.errors.values().sum();
    let stale = c.errors.get("target.stale").copied().unwrap_or(0);
    let downloads = state::downloads_snapshot();
    let finished = downloads.iter().filter(|d| d["complete"] == true).count() as u64;
    json!({
        "commands": c.commands,
        "actions": c.actions,
        "latency": {"count": sorted.len(), "p50": pct(0.5), "p90": pct(0.9), "max": sorted.last(), "mean": if sorted.is_empty() { Value::Null } else { json!(sorted.iter().sum::<u64>() / sorted.len() as u64) }},
        "batches": {"count": c.batches.len(), "meanSteps": if c.batches.is_empty() { Value::Null } else { json!(c.batches.iter().sum::<u64>() as f64 / c.batches.len() as f64) }, "maxSteps": c.batches.iter().max()},
        "errors": c.errors,
        "recovered": c.recovered,
        "staleReferenceRate": ratio(stale, c.actions),
        "handoffs": {"count": c.handoffs.len(), "rate": ratio(c.handoffs.len() as u64, c.actions), "meanResolutionMs": if c.handoffs.is_empty() { Value::Null } else { json!(c.handoffs.iter().sum::<u64>() / c.handoffs.len() as u64) }},
        "policy": {"approvalRequired": c.errors.get("approval.required").copied().unwrap_or(0), "denied": c.errors.get("policy.denied").copied().unwrap_or(0), "handoffRequired": c.errors.get("handoff.required").copied().unwrap_or(0)},
        "uploads": {"ok": c.uploads.0, "failed": c.uploads.1, "rate": ratio(c.uploads.0, c.uploads.0 + c.uploads.1)},
        "dialogs": {"ok": c.dialogs.0, "failed": c.dialogs.1, "rate": ratio(c.dialogs.0, c.dialogs.0 + c.dialogs.1)},
        "downloads": {"finished": finished, "total": downloads.len(), "rate": ratio(finished, downloads.len() as u64)},
        "replyTokensEstimate": c.reply_chars / 4,
        "errorRate": ratio(errors, c.commands),
        "tasks": {
            "reported": c.tasks.len(),
            "succeeded": succeeded,
            "successRate": ratio(succeeded, c.tasks.len() as u64),
            "falseCompletions": c.tasks.iter().filter(|task| task["falseCompletion"] == true).count(),
            "falseCompletionRate": ratio(c.tasks.iter().filter(|task| task["falseCompletion"] == true).count() as u64, c.tasks.len() as u64),
            "tokens": tokens,
            "successPerThousandTokens": if tokens > 0.0 { json!((succeeded as f64 / tokens * 1000.0 * 1000.0).round() / 1000.0) } else { Value::Null },
            "modelTurns": turns,
            "extractionAccuracy": if accuracy.is_empty() { Value::Null } else { json!(accuracy.iter().sum::<f64>() / accuracy.len() as f64) },
        },
    })
}

/// Every `metrics.*` command. `metrics.success` with `ok` records the outcome
/// of a task as the planner or evaluator judged it (optionally with `tokens`,
/// `turns`, `accuracy`, `falseCompletion`).
pub fn metrics(session: &str, method: &str, params: &Value) -> Reply {
    if method == "metrics.success" && params.get("ok").is_some() {
        let task = json!({
            "task": params["task"].as_str().unwrap_or("task"), "ok": params["ok"] == true,
            "tokens": params["tokens"].as_f64(), "turns": params["turns"].as_f64(),
            "accuracy": params["accuracy"].as_f64(), "falseCompletion": params["falseCompletion"] == true, "at": now(),
        });
        METRICS.with(|all| {
            all.borrow_mut()
                .entry(session.to_string())
                .or_default()
                .tasks
                .push(task)
        });
    }
    if method == "metrics.reset" {
        METRICS.with(|all| all.borrow_mut().remove(session));
    }
    let all = snapshot(&counters(session));
    let part = match method {
        "metrics.latency" => all["latency"].clone(),
        "metrics.batch_size" => all["batches"].clone(),
        "metrics.recovery_count" => json!({"recovered": all["recovered"], "errors": all["errors"]}),
        "metrics.stale_reference_rate" => {
            json!({"rate": all["staleReferenceRate"], "actions": all["actions"]})
        }
        "metrics.handoff_rate" | "metrics.handoff_resolution_time" => all["handoffs"].clone(),
        "metrics.native_dialog_success" => all["dialogs"].clone(),
        "metrics.upload_success" => all["uploads"].clone(),
        "metrics.download_success" => all["downloads"].clone(),
        "metrics.model_turns" => json!({"turns": all["tasks"]["modelTurns"]}),
        "metrics.success_per_token" => {
            json!({"successPerThousandTokens": all["tasks"]["successPerThousandTokens"], "tokens": all["tasks"]["tokens"]})
        }
        "metrics.false_completion_rate" => json!({"rate": all["tasks"]["falseCompletionRate"]}),
        "metrics.extraction_accuracy" => json!({"accuracy": all["tasks"]["extractionAccuracy"]}),
        _ => all["tasks"].clone(),
    };
    Ok(json!({"metric": method, "value": part, "all": all}))
}

fn bench_dir() -> PathBuf {
    state::state().options.cache_root.join("benchmarks")
}

fn bench_file(name: &str) -> Result<PathBuf, Fail> {
    if name.is_empty()
        || name.len() > 64
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(Fail::params(
            "a benchmark name is 1 to 64 letters, digits, - or _",
        ));
    }
    Ok(bench_dir().join(format!("{name}.json")))
}

fn load_bench(name: &str) -> Result<Value, Fail> {
    let file = bench_file(name)?;
    serde_json::from_slice(
        &std::fs::read(&file).map_err(|_| Fail::params(format!("no benchmark named {name}")))?,
    )
    .map_err(|_| Fail::internal("the benchmark file is damaged"))
}

/// Key figures of a report, flattened (lower is better unless noted).
fn figures(report: &Value) -> Vec<(&'static str, Option<f64>, bool)> {
    let m = &report["metrics"];
    vec![
        ("latency.p50", m["latency"]["p50"].as_f64(), false),
        ("latency.p90", m["latency"]["p90"].as_f64(), false),
        ("errorRate", m["errorRate"].as_f64(), false),
        (
            "staleReferenceRate",
            m["staleReferenceRate"].as_f64(),
            false,
        ),
        (
            "replyTokensEstimate",
            m["replyTokensEstimate"].as_f64(),
            false,
        ),
        (
            "tasks.successRate",
            m["tasks"]["successRate"].as_f64(),
            true,
        ),
        ("tasks.tokens", m["tasks"]["tokens"].as_f64(), false),
        ("tasks.modelTurns", m["tasks"]["modelTurns"].as_f64(), false),
    ]
}

/// `benchmark.record|compare|regression|token_report|recovery_report|policy_report`.
pub fn benchmark(session: &str, method: &str, params: &Value) -> Reply {
    let current = snapshot(&counters(session));
    match method {
        "benchmark.record" => {
            let name = params["name"]
                .as_str()
                .ok_or_else(|| Fail::params("params.name is required"))?;
            let file = bench_file(name)?;
            std::fs::create_dir_all(bench_dir()).map_err(|e| Fail::internal(e.to_string()))?;
            let report = json!({"name": name, "at": now(), "notes": params.get("notes"), "metrics": current});
            std::fs::write(
                &file,
                serde_json::to_vec_pretty(&report).unwrap_or_default(),
            )
            .map_err(|e| Fail::internal(e.to_string()))?;
            Ok(json!({"name": name, "file": file.display().to_string(), "metrics": current}))
        }
        "benchmark.compare" | "benchmark.regression" => {
            let (a, b) = if method == "benchmark.compare" {
                (
                    load_bench(params["a"].as_str().unwrap_or(""))?,
                    load_bench(params["b"].as_str().unwrap_or(""))?,
                )
            } else {
                let baseline = load_bench(params["baseline"].as_str().unwrap_or(""))?;
                let now_report = match params["name"].as_str() {
                    Some(name) => load_bench(name)?,
                    None => json!({"metrics": current}),
                };
                (baseline, now_report)
            };
            let threshold = params["threshold"].as_f64().unwrap_or(0.10);
            let mut rows = Vec::new();
            let mut regressions = Vec::new();
            for ((name, before, higher_better), (_, after, _)) in
                figures(&a).into_iter().zip(figures(&b))
            {
                let change = match (before, after) {
                    (Some(x), Some(y)) if x != 0.0 => Some((y - x) / x.abs()),
                    _ => None,
                };
                let worse = change.is_some_and(|c| {
                    if higher_better {
                        c < -threshold
                    } else {
                        c > threshold
                    }
                });
                if worse {
                    regressions.push(name);
                }
                rows.push(json!({"figure": name, "before": before, "after": after, "change": change.map(|c| (c * 1000.0).round() / 1000.0), "worse": worse}));
            }
            Ok(
                json!({"figures": rows, "regressions": regressions, "regressed": !regressions.is_empty(), "threshold": threshold}),
            )
        }
        "benchmark.token_report" => Ok(
            json!({"replyTokensEstimate": current["replyTokensEstimate"], "commands": current["commands"], "perCommand": current["commands"].as_u64().filter(|n| *n > 0).map(|n| current["replyTokensEstimate"].as_u64().unwrap_or(0) / n), "planner": {"tokens": current["tasks"]["tokens"], "turns": current["tasks"]["modelTurns"]}}),
        ),
        "benchmark.recovery_report" => Ok(
            json!({"errors": current["errors"], "recovered": current["recovered"], "staleReferenceRate": current["staleReferenceRate"]}),
        ),
        _ => Ok(json!({"policy": current["policy"], "handoffs": current["handoffs"]})),
    }
}

// ---------------------------------------------------------------------------
// memory and research cache

fn memory_file() -> PathBuf {
    state::state().options.cache_root.join("memory.json")
}

/// `memory.*`: notes kept between tasks. Scope defaults to "default".
pub fn memory(method: &str, params: &Value) -> Reply {
    let mut all: BTreeMap<String, BTreeMap<String, Value>> = std::fs::read(memory_file())
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    let scope = params["scope"].as_str().unwrap_or("default").to_string();
    let key = params["key"].as_str().map(str::to_string);
    let mut changed = false;
    let out = match method {
        "memory.store" => {
            let key = key.ok_or_else(|| Fail::params("params.key is required"))?;
            let value = params
                .get("value")
                .cloned()
                .ok_or_else(|| Fail::params("params.value is required"))?;
            if value.to_string().len() > 100_000 {
                return Err(Fail::params("a memory value is limited to 100 KB"));
            }
            all.entry(scope.clone())
                .or_default()
                .insert(key.clone(), json!({"value": value, "at": now()}));
            changed = true;
            json!({"scope": scope, "key": key, "stored": true})
        }
        "memory.retrieve" => {
            let items = all.get(&scope).cloned().unwrap_or_default();
            match (key, params["query"].as_str()) {
                (Some(key), _) => {
                    json!({"scope": scope, "key": key, "value": items.get(&key).map(|item| item["value"].clone())})
                }
                (None, Some(query)) => {
                    let q = query.to_ascii_lowercase();
                    json!({"scope": scope, "items": items.iter().filter(|(k, v)| k.to_ascii_lowercase().contains(&q) || v["value"].to_string().to_ascii_lowercase().contains(&q)).map(|(k, v)| json!({"key": k, "value": v["value"]})).collect::<Vec<_>>()})
                }
                _ => {
                    json!({"scope": scope, "items": items.iter().map(|(k, v)| json!({"key": k, "value": v["value"]})).collect::<Vec<_>>()})
                }
            }
        }
        "memory.forget" => {
            let removed = match key {
                Some(key) => all
                    .get_mut(&scope)
                    .and_then(|items| items.remove(&key))
                    .is_some(),
                None => all.remove(&scope).is_some(),
            };
            changed = true;
            json!({"scope": scope, "forgotten": removed})
        }
        "memory.scope" => {
            json!({"scopes": all.iter().map(|(name, items)| json!({"scope": name, "keys": items.keys().collect::<Vec<_>>()})).collect::<Vec<_>>()})
        }
        "memory.redact" => {
            let key = key.ok_or_else(|| Fail::params("params.key is required"))?;
            let item = all
                .get_mut(&scope)
                .and_then(|items| items.get_mut(&key))
                .ok_or_else(|| Fail::params(format!("nothing stored as {key}")))?;
            item["value"] = json!("[redacted]");
            changed = true;
            json!({"scope": scope, "key": key, "redacted": true})
        }
        "memory.summarize" => {
            json!({"scopes": all.iter().map(|(name, items)| json!({"scope": name, "count": items.len(), "items": items.iter().map(|(k, v)| json!({"key": k, "preview": v["value"].to_string().chars().take(80).collect::<String>(), "at": v["at"]})).collect::<Vec<_>>()})).collect::<Vec<_>>()})
        }
        "memory.export" => json!({"memory": all}),
        _ => {
            // memory.import {memory: {scope: {key: value}}}
            let incoming = params["memory"]
                .as_object()
                .ok_or_else(|| Fail::params("params.memory: {scope: {key: value}}"))?;
            let mut count = 0;
            for (scope, items) in incoming {
                for (key, value) in items.as_object().into_iter().flatten() {
                    let value = value.get("value").cloned().unwrap_or_else(|| value.clone());
                    all.entry(scope.clone())
                        .or_default()
                        .insert(key.clone(), json!({"value": value, "at": now()}));
                    count += 1;
                }
            }
            changed = true;
            json!({"imported": count})
        }
    };
    if changed {
        std::fs::write(
            memory_file(),
            serde_json::to_vec_pretty(&all).unwrap_or_default(),
        )
        .map_err(|e| Fail::internal(e.to_string()))?;
    }
    Ok(out)
}

/// `research_cache.get|put|invalidate`: data already fetched, by address
/// (kept while the browser runs).
pub fn research_cache(method: &str, params: &Value) -> Reply {
    let url = params["url"].as_str().map(str::to_string);
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        match method {
            "research_cache.put" => {
                let url = url.ok_or_else(|| Fail::params("params.url is required"))?;
                if cache.len() >= 2000 {
                    cache.clear();
                }
                cache.insert(
                    url.clone(),
                    (now(), params.get("data").cloned().unwrap_or(Value::Null)),
                );
                Ok(json!({"url": url, "cached": true}))
            }
            "research_cache.get" => {
                let url = url.ok_or_else(|| Fail::params("params.url is required"))?;
                Ok(match cache.get(&url) {
                    Some((at, data)) => {
                        json!({"url": url, "hit": true, "ageSeconds": now() - at, "data": data})
                    }
                    None => json!({"url": url, "hit": false}),
                })
            }
            _ => {
                let removed = match url {
                    Some(url) => usize::from(cache.remove(&url).is_some()),
                    None => {
                        let n = cache.len();
                        cache.clear();
                        n
                    }
                };
                Ok(json!({"invalidated": removed}))
            }
        }
    })
}

// ---------------------------------------------------------------------------
// sessions, export and checkpoints

fn sessions_file() -> PathBuf {
    state::state()
        .options
        .cache_root
        .join("saved-sessions.json")
}

fn open_pages() -> Vec<String> {
    state::content_tab_ids()
        .iter()
        .map(|id| state::tab_url(id))
        .filter(|url| url.starts_with("http") || url.starts_with("file:"))
        .collect()
}

/// `profile.session_save|session_list|session_resume`.
pub fn saved_sessions(method: &str, params: &Value) -> Reply {
    let mut all: BTreeMap<String, Value> = std::fs::read(sessions_file())
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    match method {
        "profile.session_list" => Ok(
            json!({"sessions": all.iter().map(|(name, s)| json!({"name": name, "pages": s["pages"].as_array().map_or(0, Vec::len), "at": s["at"]})).collect::<Vec<_>>()}),
        ),
        "profile.session_save" => {
            let name = params["name"]
                .as_str()
                .filter(|n| !n.is_empty() && n.len() <= 64)
                .ok_or_else(|| Fail::params("params.name is required"))?;
            let pages = open_pages();
            all.insert(name.to_string(), json!({"pages": pages, "at": now()}));
            std::fs::write(
                sessions_file(),
                serde_json::to_vec_pretty(&all).unwrap_or_default(),
            )
            .map_err(|e| Fail::internal(e.to_string()))?;
            Ok(json!({"name": name, "pages": pages}))
        }
        _ => {
            let name = params["name"].as_str().unwrap_or("");
            let pages: Vec<String> = all
                .get(name)
                .and_then(|s| s["pages"].as_array().cloned())
                .ok_or_else(|| Fail::params(format!("no saved session {name}")))?
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect();
            let mut opened = Vec::new();
            for page in &pages {
                if let Ok(tab) =
                    state::create_content_tab_placed(page, state::agent_tab_placement())
                {
                    opened.push(tab["tabId"].clone());
                }
            }
            Ok(json!({"name": name, "opened": opened}))
        }
    }
}

fn checkpoint_dir() -> PathBuf {
    state::state().options.cache_root.join("checkpoints")
}

/// The parts of a profile that can be exported: bookmarks, settings (minus
/// private keys), saved sessions and recent history. Never passwords,
/// cookies or tokens.
fn export_value() -> Value {
    let mut settings = state::settings_snapshot();
    if let Some(map) = settings.as_object_mut() {
        for private in ["sitePermissions", "syncFolder", "uploadDirs", "profileId"] {
            map.remove(private);
        }
    }
    json!({
        "app": "AgentGo", "kind": "profile-state", "version": 1, "at": now(),
        "bookmarks": state::with_store(|store| store.bookmarks(5000)).unwrap_or_default(),
        "settings": settings,
        "sessions": std::fs::read(sessions_file()).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()),
        "history": state::history_snapshot(500),
    })
}

fn import_value(data: &Value) -> Result<Value, Fail> {
    if data["kind"] != "profile-state" {
        return Err(Fail::params("this is not an AgentGo profile export"));
    }
    let mut bookmarks = 0;
    for bookmark in data["bookmarks"].as_array().cloned().unwrap_or_default() {
        if let (Some(url), Some(title)) = (bookmark["url"].as_str(), bookmark["title"].as_str()) {
            if state::is_navigable_url(url)
                && state::with_store(|store| {
                    store.add_bookmark(url, title, bookmark["createdAt"].as_u64().unwrap_or(0))
                })
                .is_ok()
            {
                bookmarks += 1;
            }
        }
    }
    if let Some(settings) = data["settings"].as_object() {
        let _ = state::update_settings(&Value::Object(settings.clone()));
    }
    if let Some(sessions) = data.get("sessions").filter(|s| s.is_object()) {
        let _ = std::fs::write(
            sessions_file(),
            serde_json::to_vec_pretty(sessions).unwrap_or_default(),
        );
    }
    Ok(json!({"bookmarks": bookmarks, "settings": data["settings"].is_object()}))
}

/// `profile.export_state|import_state|checkpoint|restore_checkpoint` (operator).
pub fn profile_state(method: &str, params: &Value) -> Reply {
    let named = |name: &str| -> Result<PathBuf, Fail> {
        if name.is_empty()
            || name.len() > 64
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(Fail::params(
                "a checkpoint name is 1 to 64 letters, digits, - or _",
            ));
        }
        Ok(checkpoint_dir().join(format!("{name}.json")))
    };
    match method {
        "profile.export_state" | "profile.checkpoint" => {
            let file = match params["path"].as_str() {
                Some(path) if method == "profile.export_state" => PathBuf::from(path),
                _ => named(params["name"].as_str().unwrap_or(
                    if method == "profile.export_state" {
                        "export"
                    } else {
                        ""
                    },
                ))?,
            };
            if let Some(parent) = file.parent() {
                std::fs::create_dir_all(parent).map_err(|e| Fail::internal(e.to_string()))?;
            }
            let value = export_value();
            std::fs::write(&file, serde_json::to_vec_pretty(&value).unwrap_or_default())
                .map_err(|e| Fail::internal(e.to_string()))?;
            Ok(
                json!({"file": file.display().to_string(), "bookmarks": value["bookmarks"].as_array().map_or(0, Vec::len)}),
            )
        }
        _ => {
            let file = match params["path"].as_str() {
                Some(path) if method == "profile.import_state" => PathBuf::from(path),
                _ => named(params["name"].as_str().unwrap_or(""))?,
            };
            let data: Value = serde_json::from_slice(
                &std::fs::read(&file)
                    .map_err(|_| Fail::params(format!("cannot read {}", file.display())))?,
            )
            .map_err(|_| Fail::params("the file is not JSON"))?;
            import_value(&data)
        }
    }
}

// ---------------------------------------------------------------------------
// domain policy

fn domains(key: &str) -> Vec<String> {
    state::settings_snapshot()[key]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn host_matches(host: &str, rule: &str) -> bool {
    let rule = rule.trim_start_matches("*.").to_ascii_lowercase();
    host == rule || host.ends_with(&format!(".{rule}"))
}

/// Whether agents may use a page on `host` under the person's domain lists.
pub fn domain_allowed(host: &str) -> Result<(), Fail> {
    let host = host.to_ascii_lowercase();
    if domains("agentBlockedDomains")
        .iter()
        .any(|rule| host_matches(&host, rule))
    {
        return Err(Fail::new(
            ErrorCode::PolicyDenied,
            format!("{host} is on the person's list of sites agents may not use"),
        ));
    }
    let allowed = domains("agentAllowedDomains");
    if !allowed.is_empty() && !allowed.iter().any(|rule| host_matches(&host, rule)) {
        return Err(Fail::new(
            ErrorCode::PolicyDenied,
            format!("agents may only use the sites the person allowed; {host} is not one of them"),
        ));
    }
    Ok(())
}

/// `policy.domain_blocklist` / `policy.domain_allowlist`: without `add` or
/// `remove` it lists; changes are operator only (checked by the caller).
pub fn domain_list(method: &str, params: &Value) -> Reply {
    let key = if method == "policy.domain_blocklist" {
        "agentBlockedDomains"
    } else {
        "agentAllowedDomains"
    };
    let mut list = domains(key);
    let valid = |d: &str| {
        !d.is_empty()
            && d.len() < 254
            && d.trim_start_matches("*.")
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
    };
    if let Some(add) = params["add"].as_str() {
        if !valid(add) {
            return Err(Fail::params("a domain like example.com or *.example.com"));
        }
        if !list.iter().any(|d| d == add) {
            list.push(add.to_ascii_lowercase());
        }
    }
    if let Some(remove) = params["remove"].as_str() {
        list.retain(|d| d != remove);
    }
    if params.get("add").is_some() || params.get("remove").is_some() {
        state::update_settings(&json!({ key: list })).map_err(Fail::internal)?;
    }
    Ok(
        json!({"list": if key == "agentBlockedDomains" { "blocked" } else { "allowed" }, "domains": list}),
    )
}

// ---------------------------------------------------------------------------
// artifacts and files

fn artifacts_root() -> PathBuf {
    state::state()
        .paths
        .artifact_dir("x")
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default()
}

/// A path that must stay inside AgentGo's artifact or download folders.
fn inside(path: &str) -> Result<PathBuf, Fail> {
    let candidate = PathBuf::from(path);
    let resolved = candidate
        .canonicalize()
        .or_else(|_| {
            candidate
                .parent()
                .and_then(|p| p.canonicalize().ok())
                .map(|p| p.join(candidate.file_name().unwrap_or_default()))
                .ok_or(())
        })
        .map_err(|_| Fail::params(format!("{path} does not exist")))?;
    let roots = [artifacts_root(), state::download_root()];
    if roots
        .iter()
        .filter_map(|root| root.canonicalize().ok())
        .any(|root| resolved.starts_with(root))
    {
        Ok(resolved)
    } else {
        Err(Fail::new(
            ErrorCode::PolicyDenied,
            "agents work with files only inside AgentGo's artifact and download folders",
        ))
    }
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, out)
        } else {
            out.push(path)
        }
    }
}

fn describe(path: &Path) -> Value {
    let meta = std::fs::metadata(path).ok();
    json!({
        "path": path.display().to_string(),
        "name": path.file_name().map(|n| n.to_string_lossy().to_string()),
        "bytes": meta.as_ref().map(|m| m.len()),
        "modified": meta.and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()),
    })
}

fn sha256(bytes: &[u8]) -> String {
    agentgo_artifacts::sha256_hex(bytes)
}

/// `artifact.*` and the scoped `file.*` commands.
pub fn artifacts(session: &str, method: &str, params: &Value) -> Reply {
    let path = || {
        params["path"]
            .as_str()
            .ok_or_else(|| Fail::params("params.path is required"))
            .and_then(inside)
    };
    let mut files = Vec::new();
    walk(&artifacts_root(), &mut files);
    match method {
        "artifact.list" => Ok(
            json!({"artifacts": files.iter().rev().take(params["max"].as_u64().unwrap_or(100) as usize).map(|p| describe(p)).collect::<Vec<_>>(), "total": files.len()}),
        ),
        "artifact.search" => {
            let needle = params["text"].as_str().unwrap_or("").to_ascii_lowercase();
            Ok(
                json!({"artifacts": files.iter().filter(|p| p.to_string_lossy().to_ascii_lowercase().contains(&needle)).take(100).map(|p| describe(p)).collect::<Vec<_>>()}),
            )
        }
        "artifact.metadata" => {
            let p = path()?;
            let mut out = describe(&p);
            out["sha256"] = json!(sha256(&std::fs::read(&p).unwrap_or_default()));
            Ok(out)
        }
        "artifact.hash" => {
            let p = path()?;
            Ok(
                json!({"path": p.display().to_string(), "sha256": sha256(&std::fs::read(&p).map_err(|e| Fail::internal(e.to_string()))?)}),
            )
        }
        "artifact.read" | "artifact.preview" => {
            let p = path()?;
            let bytes = std::fs::read(&p).map_err(|e| Fail::internal(e.to_string()))?;
            let max = params["max"]
                .as_u64()
                .unwrap_or(if method == "artifact.preview" {
                    2000
                } else {
                    200_000
                }) as usize;
            Ok(match String::from_utf8(bytes.clone()) {
                Ok(text) => {
                    json!({"path": p.display().to_string(), "text": text.chars().take(max).collect::<String>(), "bytes": bytes.len()})
                }
                Err(_) => {
                    json!({"path": p.display().to_string(), "binary": true, "bytes": bytes.len(), "link": format!("file:///{}", p.display().to_string().replace('\\', "/"))})
                }
            })
        }
        "artifact.link" => {
            let p = path()?;
            Ok(json!({"link": format!("file:///{}", p.display().to_string().replace('\\', "/"))}))
        }
        "artifact.delete" => {
            let p = path()?;
            std::fs::remove_file(&p).map_err(|e| Fail::internal(e.to_string()))?;
            Ok(json!({"deleted": p.display().to_string()}))
        }
        "artifact.create" | "artifact.store" | "file.write_text" => {
            let text = params["text"]
                .as_str()
                .ok_or_else(|| Fail::params("params.text is required"))?;
            if text.len() > 5_000_000 {
                return Err(Fail::params("limited to 5 MB"));
            }
            let target = match params["path"].as_str() {
                Some(_) if method == "file.write_text" => {
                    let candidate = PathBuf::from(params["path"].as_str().unwrap_or(""));
                    inside(
                        candidate
                            .parent()
                            .map(|p| p.display().to_string())
                            .unwrap_or_default()
                            .as_str(),
                    )?;
                    candidate
                }
                _ => {
                    let extension: String = params["extension"]
                        .as_str()
                        .unwrap_or("txt")
                        .chars()
                        .filter(char::is_ascii_alphanumeric)
                        .take(8)
                        .collect();
                    state::new_artifact_path(session, "note", &extension)
                        .map_err(|e| Fail::internal(e.to_string()))?
                }
            };
            std::fs::write(&target, text).map_err(|e| Fail::internal(e.to_string()))?;
            Ok(describe(&target))
        }
        "artifact.redact" => {
            let p = path()?;
            let text = std::fs::read_to_string(&p)
                .map_err(|_| Fail::params("only text artifacts can be redacted"))?;
            let mut redacted = String::with_capacity(text.len());
            let mut count = 0;
            for word in text.split_inclusive(char::is_whitespace) {
                let core = word.trim_end();
                let digits = core.chars().filter(char::is_ascii_digit).count();
                if (core.contains('@') && core.contains('.')) || digits >= 9 {
                    redacted.push_str("[redacted]");
                    redacted.push_str(&word[core.len()..]);
                    count += 1;
                } else {
                    redacted.push_str(word);
                }
            }
            std::fs::write(&p, redacted).map_err(|e| Fail::internal(e.to_string()))?;
            Ok(json!({"path": p.display().to_string(), "redacted": count}))
        }
        "artifact.deduplicate" => {
            let mut seen = HashMap::new();
            let mut removed = Vec::new();
            for file in &files {
                let hash = sha256(&std::fs::read(file).unwrap_or_default());
                if seen.insert(hash, file.clone()).is_some() && std::fs::remove_file(file).is_ok() {
                    removed.push(file.display().to_string());
                }
            }
            Ok(json!({"removed": removed}))
        }
        "artifact.retention" | "artifact.expire" => {
            let days = params["days"]
                .as_f64()
                .unwrap_or(if method == "artifact.expire" {
                    0.0
                } else {
                    30.0
                })
                .max(0.0);
            let cutoff = now().saturating_sub((days * 86_400.0) as u64);
            let mut removed = 0;
            for file in &files {
                let old = std::fs::metadata(file)
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .is_some_and(|d| d.as_secs() <= cutoff);
                if old && std::fs::remove_file(file).is_ok() {
                    removed += 1;
                }
            }
            Ok(json!({"olderThanDays": days, "removed": removed}))
        }
        "artifact.bundle" | "file.archive" => {
            let list: Vec<PathBuf> = params["paths"]
                .as_array()
                .ok_or_else(|| Fail::params("params.paths: the files to put in the archive"))?
                .iter()
                .filter_map(Value::as_str)
                .map(inside)
                .collect::<Result<_, _>>()?;
            let target = state::new_artifact_path(session, "bundle", "zip")
                .map_err(|e| Fail::internal(e.to_string()))?;
            let quoted: Vec<String> = list
                .iter()
                .map(|p| format!("'{}'", p.display().to_string().replace('\'', "''")))
                .collect();
            let script = format!(
                "Compress-Archive -LiteralPath {} -DestinationPath '{}' -Force",
                quoted.join(","),
                target.display().to_string().replace('\'', "''")
            );
            let ok = super::exec::hidden_command("powershell")
                .args(["-NoProfile", "-NonInteractive", "-Command", &script])
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if !ok {
                return Err(Fail::internal("the archive could not be created"));
            }
            Ok(describe(&target))
        }
        "file.extract_archive" => {
            let archive = path()?;
            let target = state::new_artifact_path(session, "extracted", "d")
                .map_err(|e| Fail::internal(e.to_string()))?;
            let bytes = std::fs::read(&archive).map_err(|e| Fail::internal(e.to_string()))?;
            super::pack::extract_zip(&bytes, &target).map_err(|reason| {
                Fail::internal(format!("the archive could not be extracted: {reason}"))
            })?;
            let mut out = Vec::new();
            walk(&target, &mut out);
            Ok(
                json!({"folder": target.display().to_string(), "files": out.iter().map(|p| p.display().to_string()).collect::<Vec<_>>()}),
            )
        }
        _ => {
            // file.copy / file.move / file.rename: both ends inside AgentGo's folders.
            let from = path()?;
            let to_text = params["to"]
                .as_str()
                .ok_or_else(|| Fail::params("params.to is required"))?;
            let to = if method == "file.rename" {
                let name = to_text.replace(['/', '\\'], "");
                from.with_file_name(name)
            } else {
                PathBuf::from(to_text)
            };
            inside(
                to.parent()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default()
                    .as_str(),
            )?;
            if method == "file.copy" {
                std::fs::copy(&from, &to).map_err(|e| Fail::internal(e.to_string()))?;
            } else {
                std::fs::rename(&from, &to).map_err(|e| Fail::internal(e.to_string()))?;
            }
            Ok(describe(&to))
        }
    }
}
