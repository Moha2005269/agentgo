//! The single command dispatcher.
//!
//! Stdio, the named pipe and the trusted browser chrome all enter here, on
//! the CEF UI thread, so capability, lease and policy behavior cannot differ
//! between adapters. Handlers are async: a command may span many DevTools
//! round trips while the browser keeps rendering.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};

use agentgo_protocol::{ErrorCode, Request, Response};
use cef::{ImplCookieManager, ImplRequestContext};
use serde_json::{json, Value};

use super::sessions::{self, Caller, Control, Fail, Reply};
use super::{
    accounts, cdp, decision, dialogs, exec, extras, inspect, lifecycle, misc, native, page,
    profiles, recorder, sdk, shell, skills, state, suite, tasks, winmgr,
};

/// Replies remembered per (session, idempotency key), with their order of arrival.
type Replays = (HashMap<(String, String), Reply>, VecDeque<(String, String)>);

thread_local! {
    /// Addresses of the last closed tabs, newest last, for tab.reopen_closed.
    static CLOSED: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    /// Replies remembered per (session, idempotency key) so a retried
    /// side-effecting request is answered without being executed twice.
    static IDEMPOTENT: RefCell<Replays> =
        RefCell::new((HashMap::new(), VecDeque::new()));
}

/// Commands that never change page, browser or user state. Everything else
/// passes the operator's pause/stop gate and is written to the audit log.
fn read_only(method: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "api.",
        "recovery.",
        "observe.",
        "state.",
        "wait.",
        "events.",
        "siteinfo.",
        "search.",
        "locate.",
        "ground.",
    ];
    const EXACT: &[&str] = &[
        "agent.connect",
        "agent.disconnect",
        "decision.status",
        "connectors.list",
        "credentials.list",
        "clipboard.read",
        "accounts.list",
        "browser.health",
        "security.status",
        "browser.version",
        "browser.capabilities",
        "browser.crash_report",
        "window.status",
        "tabs.list",
        "tab.list",
        "tabs.status",
        "tab.status",
        "history.list",
        "downloads.list",
        "download.list",
        "download.progress",
        "settings.get",
        "profile.list",
        "agent.status",
        "approval.wait",
        "handoff.wait",
        "dialog.list",
        "network.requests",
        "network.failures",
        "network.observe_responses",
        "runtime.console",
        "storage.cookies_list",
        "storage.cookie_get",
        "form.discover",
        "session.info",
        "agent.monitor",
        "agent.preview",
        "agent.dry_run",
        "agent.explain",
        "agent.compare",
        "agent.merge_results",
        "agent.deduplicate_results",
        "batch.preview",
        "batch.transaction_status",
        "batch.idempotency_key",
    ];
    PREFIXES.iter().any(|prefix| method.starts_with(prefix)) || EXACT.contains(&method)
}

/// Whether a command only reads (for the recorder).
pub(super) fn is_read_only(method: &str) -> bool {
    read_only(method)
}

/// Settings only the human operator may change: they widen what agents can do,
/// or they are the person's own browser preferences.
const OPERATOR_SETTINGS: [&str; 15] = [
    "vpn",
    "adBlock",
    "fingerprintProtection",
    "nativeControl",
    "sitePermissions",
    "searchEngine",
    "startup",
    "languages",
    "layaEnabled",
    "downloadsEnabled",
    "controlMode",
    "uploadDirs",
    "allowEvaluate",
    "profileId",
    "agentTabs",
];

fn shell_only(caller: &Caller, method: &str) -> Result<(), Fail> {
    if caller.is_shell() {
        Ok(())
    } else {
        Err(Fail::new(
            ErrorCode::PolicyDenied,
            format!("{method} is reserved for the human operator"),
        ))
    }
}

fn legacy(method: &str, params: Value) -> Reply {
    let response = state::handle_protocol_request(Request::new(0, method, params));
    match (response.ok, response.result, response.error) {
        (true, result, _) => Ok(result.unwrap_or(Value::Null)),
        (false, _, Some(error)) => Err(Fail {
            code: ErrorCode::from_wire(&error.code).unwrap_or(ErrorCode::Internal),
            message: error.message,
            data: error.data,
        }),
        (false, _, None) => Err(Fail::internal("handler returned no error detail")),
    }
}

/// The reply for a navigation whose site has not answered by the deadline.
fn not_answered(tab: &str, url: &str, ms: u64) -> Value {
    json!({
        "tabId": tab,
        "url": url,
        "loaded": false,
        "committed": false,
        "ms": ms,
        "note": "the site has not answered yet; call wait.load to keep waiting, or tab.stop to give up",
    })
}

/// Navigates and waits for the load to finish, optionally returning a view
/// of the new page in the same reply (one model turn instead of three).
async fn navigate_open(caller: &Caller, params: &Value) -> Reply {
    let tab = page::tab_of(caller, params)?;
    if !caller.is_shell() && sessions::handoff(&tab).is_some() {
        // Leaving the page is not solving its check: it gives up on it.
        sessions::withdraw_handoff(&tab);
    }
    let input = params
        .get("url")
        .and_then(Value::as_str)
        .or_else(|| params.get("query").and_then(Value::as_str))
        .ok_or_else(|| Fail::params("navigation needs params.url (or params.query to search)"))?;
    let url = state::resolve_search_input(input);
    if !state::is_navigable_url(&url) {
        return Err(Fail::params("url must use http, https, file, or about"));
    }
    let timeout = params
        .get("timeout")
        .and_then(Value::as_u64)
        .unwrap_or(20_000)
        .clamp(500, 120_000);
    let generation = cdp::generation(&tab);
    state::navigate_tab(&tab, &url);
    let started = std::time::Instant::now();
    let elapsed = |started: &std::time::Instant| started.elapsed().as_millis() as u64;
    // Wait for the navigation to commit, then for the load to finish.
    while cdp::generation(&tab) == generation && elapsed(&started) < timeout.min(10_000) {
        exec::sleep(25).await;
    }
    while state::tab_is_loading(&tab) && elapsed(&started) < timeout {
        exec::sleep(40).await;
    }
    let loaded = !state::tab_is_loading(&tab) && cdp::generation(&tab) != generation;
    if state::tab_is_loading(&tab) && !cdp::readable(&tab, 700).await {
        return Ok(not_answered(&tab, &url, elapsed(&started)));
    }
    let view = params
        .get("observe")
        .and_then(Value::as_str)
        .unwrap_or("page");
    let observed = match view {
        "none" => Ok(json!({"tabId": tab, "url": state::tab_url(&tab)})),
        "text" => {
            page::observe_view(
                caller,
                "observe.text",
                &json!({"tabId": tab, "max": params.get("max")}),
            )
            .await
        }
        _ => page::observe_page(caller, &json!({"tabId": tab, "max": params.get("max")})).await,
    };
    let mut out = match observed {
        // The site has not answered: there is no page to read yet.
        Err(fail) if fail.code == ErrorCode::Timeout && fail.message.starts_with(cdp::PENDING) => {
            return Ok(not_answered(&tab, &url, elapsed(&started)));
        }
        other => other?,
    };
    if loaded && !caller.is_shell() {
        let host = url
            .split("://")
            .nth(1)
            .and_then(|rest| rest.split(['/', '?', '#']).next())
            .unwrap_or("");
        if !host.is_empty() {
            page::show(&tab, "banner", json!([format!("Agent opened {host}")])).await;
        }
    }
    out["loaded"] = json!(loaded);
    out["ms"] = json!(elapsed(&started));
    if !loaded {
        out["note"] = json!("the page has not finished loading; what is shown may be incomplete. Call wait.load to wait longer");
    }
    if let Some(error) = state::take_load_error(&tab) {
        out["loadError"] = json!(error);
    }
    Ok(out)
}

/// `workflow.replay {name, values?, tabId?}`: runs a recording again on the
/// page as it is now. A step that acted on a reference is pointed at the
/// element whose outline line matches the recorded one; `values` replaces
/// recorded texts (`{"old text": "new text"}`), which is how a recording is
/// reused with other data. Stops at the first step that fails.
async fn replay(caller: &Caller, params: &Value) -> Reply {
    let steps = recorder::steps(params)?;
    let values = params.get("values").cloned().unwrap_or_else(|| json!({}));
    let mut done = Vec::new();
    for (index, step) in steps.iter().enumerate() {
        let method = step["method"].as_str().unwrap_or("");
        let mut step_params = step["params"].clone();
        if step["params"]["redacted"] == true {
            return Err(Fail::new(ErrorCode::TargetBlocked, format!("step {index} typed a password, which recordings never keep; sign in with credentials.fill, then replay the rest")).with(json!({"completed": done.len(), "results": done})));
        }
        if let Some(map) = step_params.as_object_mut() {
            map.remove("tabId");
            if let Some(tab) = params.get("tabId") {
                map.insert("tabId".into(), tab.clone());
            }
            if let Some(text) = map
                .get("text")
                .and_then(Value::as_str)
                .and_then(|text| values.get(text))
                .cloned()
            {
                map.insert("text".into(), text);
            }
            if let Some(line) = step["target"].as_str().filter(|_| map.contains_key("ref")) {
                let view =
                    page::observe_page(caller, &json!({"tabId": params.get("tabId"), "max": 400}))
                        .await?;
                let found = recorder::find_ref(view["page"].as_str().unwrap_or(""), line)
                    .ok_or_else(|| {
                        Fail::new(
                            ErrorCode::TargetStale,
                            format!("step {index}: {line} is not on the page any more"),
                        )
                        .with(json!({"completed": done.len(), "results": done.clone()}))
                    })?;
                map.insert("ref".into(), json!(found));
            }
        }
        match Box::pin(run(caller, method, step_params)).await {
            Ok(result) => done.push(json!({"step": index, "method": method, "target": step["target"], "ok": true, "url": result.get("url")})),
            Err(fail) => return Err(Fail::new(fail.code, format!("step {index} ({method}) failed: {}", fail.message)).with(json!({"completed": done.len(), "results": done}))),
        }
    }
    Ok(json!({"completed": done.len(), "results": done}))
}

/// `benchmark.run {recording, times}` / `benchmark.custom_suite {recordings}`:
/// replays recordings and reports how often and how fast they succeed.
async fn benchmark_run(caller: &Caller, params: &Value) -> Reply {
    let names: Vec<String> = match params.get("recordings").and_then(Value::as_array) {
        Some(list) => list
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
        None => vec![params["recording"]
            .as_str()
            .ok_or_else(|| {
                Fail::params("params.recording (or recordings for a suite) is required")
            })?
            .to_string()],
    };
    let times = params["times"].as_u64().unwrap_or(1).clamp(1, 50);
    let mut runs = Vec::new();
    for name in &names {
        for round in 0..times {
            if let Some(url) = params["startUrl"].as_str() {
                let _ = page::observe_page(caller, &json!({})).await;
                let tab = page::tab_of(caller, &json!({}))?;
                state::navigate_tab(&tab, url);
                exec::sleep(1500).await;
            }
            let started = std::time::Instant::now();
            let result = Box::pin(replay(
                caller,
                &json!({"name": name, "values": params.get("values")}),
            ))
            .await;
            runs.push(json!({"recording": name, "round": round, "ok": result.is_ok(), "ms": started.elapsed().as_millis() as u64,
                "error": result.as_ref().err().map(|fail| fail.message.clone())}));
        }
    }
    let succeeded = runs.iter().filter(|run| run["ok"] == true).count();
    Ok(
        json!({"runs": runs, "succeeded": succeeded, "total": runs.len(), "successRate": succeeded as f64 / runs.len().max(1) as f64}),
    )
}

/// Identifies the state a choice was made for: the document and its outline.
fn page_revision(tab: &str, view: &Value) -> String {
    let outline = view["page"].as_str().unwrap_or("");
    // FNV-1a: enough to tell two outlines apart, not a security boundary.
    let hash = outline
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
        });
    format!("{}-{hash:x}", cdp::generation(tab))
}

/// `workflow.search_extract`: Google search plus structured result records
/// in a single request, without any model in the loop.
/// Files an agent may look at: the download folder and the folders the
/// operator granted for uploads. Nothing else on the disk.
fn file_command(method: &str, params: &Value) -> Reply {
    let given = params["path"]
        .as_str()
        .ok_or_else(|| Fail::params(format!("{method} needs params.path")))?;
    let path = std::path::Path::new(given).canonicalize().ok();
    let allowed = path.as_ref().is_some_and(|path| {
        state::upload_allowed(given)
            || state::download_folder()
                .canonicalize()
                .is_ok_and(|dir| path.starts_with(dir))
    });
    if method == "file.path_validate" {
        return Ok(json!({"path": given, "exists": path.is_some(), "allowed": allowed}));
    }
    let (true, Some(path)) = (allowed, path) else {
        return Err(Fail::new(
            ErrorCode::PolicyDenied,
            "agents may read files only in the download folder and the folders granted with --upload-dir",
        ));
    };
    let meta = std::fs::metadata(&path).map_err(|error| Fail::internal(error.to_string()))?;
    if !meta.is_file() {
        return Err(Fail::params(format!("{given} is not a file")));
    }
    let open = || std::fs::File::open(&path).map_err(|error| Fail::internal(error.to_string()));
    match method {
        "file.metadata" => Ok(json!({
            "path": path.to_string_lossy(),
            "bytes": meta.len(),
            "modified": meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()),
        })),
        "file.hash" => Ok(json!({
            "path": path.to_string_lossy(),
            "sha256": agentgo_artifacts::sha256_hex_reader(open()?).map_err(|error| Fail::internal(error.to_string()))?,
            "bytes": meta.len(),
        })),
        _ => {
            use std::io::Read;
            let max = params["max"].as_u64().unwrap_or(100_000).min(1_000_000);
            let mut bytes = Vec::new();
            open()?
                .take(max)
                .read_to_end(&mut bytes)
                .map_err(|error| Fail::internal(error.to_string()))?;
            Ok(
                json!({"text": String::from_utf8_lossy(&bytes), "bytes": meta.len(), "truncated": meta.len() > max}),
            )
        }
    }
}

/// `scheme://host[:port]` of an http(s) address.
pub(super) fn web_origin(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    let host = rest.split(['/', '?', '#']).next()?;
    (matches!(scheme, "http" | "https") && !host.is_empty()).then(|| format!("{scheme}://{host}"))
}

/// Settings -> Privacy -> Clear browsing data. Each part defaults to on.
async fn clear_browsing_data(params: &Value) -> Reply {
    if params["confirm"] != true {
        return Err(Fail::new(
            ErrorCode::ApprovalRequired,
            "browsing_data.clear requires params.confirm=true",
        ));
    }
    let wants = |key: &str| params.get(key).and_then(Value::as_bool).unwrap_or(true);
    let mut cleared = Vec::new();
    if wants("cookies") {
        if let Some(cookies) =
            state::content_context().and_then(|context| context.cookie_manager(None))
        {
            cookies.delete_cookies(None, None, None);
            cookies.flush_store(None);
            cleared.push("cookies");
        }
    }
    if wants("siteData") {
        // ponytail: storage is cleared for the origins in recent history and
        // open tabs (DevTools has no "every origin"); a BrowsingDataRemover
        // binding would cover origins visited longer ago.
        let tab = state::active_tab_id();
        let mut origins: Vec<String> = state::history_snapshot(100)
            .iter()
            .filter_map(|item| item["url"].as_str().and_then(web_origin))
            .chain(
                state::content_tab_ids()
                    .iter()
                    .filter_map(|id| web_origin(&state::tab_url(id))),
            )
            .collect();
        origins.sort();
        origins.dedup();
        for origin in &origins {
            let _ = cdp::call(
                &tab,
                "Storage.clearDataForOrigin",
                json!({"origin": origin, "storageTypes": "all"}),
            )
            .await;
        }
        cdp::call(&tab, "Network.clearBrowserCache", json!({}))
            .await
            .map_err(Fail::internal)?;
        cleared.extend(["cache", "siteData"]);
    }
    if wants("history") {
        state::clear_history();
        cleared.push("history");
    }
    state::log_event(&format!(
        "AG-RUNTIME browsing-data-cleared {}",
        cleared.join(",")
    ));
    Ok(json!({"cleared": cleared}))
}

async fn search_extract(caller: &Caller, params: &Value) -> Reply {
    let query = params
        .get("query")
        .and_then(Value::as_str)
        .filter(|query| !query.trim().is_empty())
        .ok_or_else(|| Fail::params("workflow.search_extract needs params.query"))?;
    let limit = params
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(8)
        .clamp(1, 30);
    let opened = navigate_open(
        caller,
        // `url` names a result page directly (another engine, a later page).
        &json!({"tabId": params.get("tabId"), "url": params.get("url"), "query": query, "observe": "none"}),
    )
    .await?;
    let tab = opened["tabId"].as_str().unwrap_or_default().to_string();
    let found = page::agent(&tab, "searchResults", json!(limit), 10_000).await?;
    if found["signals"]["captcha"] == true && !caller.is_shell() {
        let id = sessions::begin_handoff(&tab, "search engine asked for human verification");
        return Err(Fail::new(
            ErrorCode::HumanHandoff,
            "the search page requires human verification; call handoff.wait and retry",
        )
        .with(json!({"handoffId": id, "tabId": tab})));
    }
    Ok(json!({"tabId": tab, "query": query, "url": opened["url"], "results": found["results"]}))
}

/// `workflow.collect`: scrolls an infinite or long page and gathers
/// de-duplicated links or text in-process.
async fn collect(caller: &Caller, params: &Value) -> Reply {
    let tab = page::tab_of(caller, params)?;
    let rounds = params
        .get("scrolls")
        .and_then(Value::as_u64)
        .unwrap_or(5)
        .clamp(1, 40);
    let limit = params
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(100)
        .clamp(1, 500) as usize;
    let mut seen = std::collections::HashSet::new();
    let mut items = Vec::new();
    for _ in 0..=rounds {
        let batch = page::agent(
            &tab,
            "links",
            json!({"filter": params.get("filter"), "max": 400}),
            10_000,
        )
        .await?;
        for link in batch["links"].as_array().into_iter().flatten() {
            let url = link["url"].as_str().unwrap_or_default().to_string();
            if seen.insert(url.clone()) {
                items.push(json!({"text": link["text"], "url": url}));
            }
        }
        if items.len() >= limit {
            break;
        }
        let before = page::agent(&tab, "scroll", json!({"pages": 0}), 5_000).await?;
        let after = page::agent(&tab, "scroll", json!({"pages": 1}), 5_000).await?;
        let _ = cdp::agent_eval(&tab, "__ag.settle(300,2500)", 6_000).await;
        if before["y"] == after["y"] && before["h"] == after["h"] {
            break; // reached the end
        }
    }
    items.truncate(limit);
    Ok(json!({"tabId": tab, "count": items.len(), "items": items}))
}

/// `workflow.batch`: runs typed steps in order inside one request. Each step
/// already waits for the page to settle, so no model turn is spent between
/// deterministic actions. Intermediate deltas are dropped unless requested.
async fn batch(caller: &Caller, params: &Value) -> Reply {
    let steps = params
        .get("steps")
        .and_then(Value::as_array)
        .filter(|steps| !steps.is_empty() && steps.len() <= 50)
        .ok_or_else(|| Fail::params("workflow.batch needs 1..50 steps: [{method, params}]"))?;
    let verbose = params
        .get("verbose")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let keep_going = params.get("stopOnError").and_then(Value::as_bool) == Some(false);
    let mut results = Vec::new();
    let mut failed = None;
    for (index, step) in steps.iter().enumerate() {
        let method = step["method"].as_str().unwrap_or_default();
        if method.is_empty() || method.starts_with("workflow.batch") {
            return Err(Fail::params(format!("step {index} has no valid method")));
        }
        let mut step_params = step.get("params").cloned().unwrap_or_else(|| json!({}));
        if let (Some(tab), Some(map)) = (params.get("tabId"), step_params.as_object_mut()) {
            map.entry("tabId").or_insert_with(|| tab.clone());
        }
        let last = index + 1 == steps.len();
        match Box::pin(run(caller, method, step_params)).await {
            Ok(mut value) => {
                if !verbose && !last && !read_only(method) {
                    // Keep only the identifying field of intermediate actions.
                    let brief: serde_json::Map<String, Value> = value
                        .as_object_mut()
                        .map(|map| {
                            [
                                "target", "field", "key", "selected", "checked", "typed", "filled",
                                "dialog", "handoff",
                            ]
                            .iter()
                            .filter_map(|key| {
                                map.remove(*key).map(|entry| (key.to_string(), entry))
                            })
                            .collect()
                        })
                        .unwrap_or_default();
                    value = Value::Object(brief);
                }
                results.push(json!({"ok": true, "result": value}));
            }
            Err(fail) => {
                results.push(json!({"ok": false, "error": {"code": fail.code.as_str(), "message": fail.message, "data": fail.data}}));
                failed.get_or_insert(index);
                if !keep_going {
                    break;
                }
            }
        }
    }
    let mut out = json!({"completed": results.iter().filter(|r| r["ok"] == true).count(), "results": results});
    if let Some(index) = failed {
        out["failedStep"] = json!(index);
    }
    Ok(out)
}

/// Routes one command. Preconditions (lease, pause/stop, operator-only
/// settings) are enforced here for every transport.
pub(super) async fn run(caller: &Caller, method: &str, mut params: Value) -> Reply {
    // Any command that takes an element reference also takes `find: {by,
    // ...}`, resolved now on the page as it is (see tasks::resolve).
    if params.get("find").is_some()
        && params.get("ref").is_none()
        && !method.starts_with("locate.")
        && !tasks::handles(method)
        && !matches!(method, "collect.paginate" | "collect.resume")
    {
        tasks::resolve(caller, &mut params).await?;
    }
    // Account Center: an agent is refused on pages where an account it was
    // not given is signed in, before the command and again on what it returns.
    if caller.is_shell() || !(accounts::checked_before(method) || accounts::checked_after(method)) {
        return run_ungated(caller, method, params).await;
    }
    if accounts::checked_before(method) {
        if let Ok(tab) = page::tab_of(caller, &params) {
            accounts::gate(&tab).await?;
        }
    }
    let named = params.clone();
    let reply = run_ungated(caller, method, params).await?;
    if accounts::checked_after(method) {
        let tab = reply["tabId"]
            .as_str()
            .map(str::to_string)
            .or_else(|| page::tab_of(caller, &named).ok());
        if let Some(tab) = tab {
            accounts::gate(&tab).await?;
        }
    }
    Ok(reply)
}

/// A register command carried out by one DevTools method on the caller's
/// tab. The spec only lists methods that change emulation or read state;
/// nothing here can click, type or run page script.
async fn devtools_tool(caller: &Caller, method: &str, defaults: &str, params: Value) -> Reply {
    let tab = page::tab_of(caller, &params)?;
    sessions::check_handoff(caller, &tab)?;
    let mut merged = serde_json::from_str::<Value>(defaults).unwrap_or_else(|_| json!({}));
    if let (Some(into), Value::Object(given)) = (merged.as_object_mut(), params) {
        into.extend(given.into_iter().filter(|(key, _)| key != "tabId"));
    }
    let out = cdp::call(&tab, method, merged).await.map_err(|message| {
        if message.contains("Invalid parameters") || message.contains("-32602") {
            Fail::params(message)
        } else {
            Fail::internal(message)
        }
    })?;
    Ok(match out {
        Value::Object(map) if map.is_empty() => json!({"accepted": true, "tabId": tab}),
        other => other,
    })
}

async fn run_ungated(caller: &Caller, method: &str, params: Value) -> Reply {
    // The process that launched the browser may always close it, even while
    // agent input is paused or stopped.
    let own_shutdown =
        method == "browser.shutdown" && state::state().options.stdio && caller.session == "stdio";
    if !caller.is_shell() && !read_only(method) && !own_shutdown {
        sessions::gate(caller).await?;
    }
    // The wider tool register: names carried out by another command, by a
    // page tool, or not built yet.
    if let Some(&(_, _, kind, target, preset)) = agentgo_shell_bridge::extra::find(method) {
        if kind == "alias" {
            let mut merged = if params.is_object() {
                params
            } else {
                json!({})
            };
            if let (Some(into), Ok(Value::Object(fixed))) = (
                merged.as_object_mut(),
                serde_json::from_str::<Value>(preset),
            ) {
                into.extend(fixed);
            }
            return Box::pin(run(caller, target, merged)).await;
        }
        if kind == "cdp" {
            return devtools_tool(caller, target, preset, params).await;
        }
        let tab = page::tab_of(caller, &params)?;
        sessions::check_handoff(caller, &tab)?;
        let out = page::agent(&tab, "tool", json!([target, params]), 10_000).await?;
        return match page::agent_error(&out) {
            Some(fail) => Err(fail),
            None => Ok(out),
        };
    }
    if let Some(reason) = agentgo_shell_bridge::extra::unsupported(method) {
        return Err(Fail::new(
            ErrorCode::CapabilityUnsupported,
            format!("{method} is not available in AgentGo: {reason}"),
        ));
    }
    if agentgo_shell_bridge::extra::planned(method) {
        return Err(Fail::new(
            ErrorCode::CapabilityUnsupported,
            format!("{method} is in the tool register but is not built yet; browser.capabilities lists what works"),
        ));
    }
    match method {
        // --- the task engine, panels, profiles, windows, dialogs ----------
        m if tasks::handles(m) => tasks::command(caller, m, &params).await,
        m if profiles::handles(m) => profiles::command(caller, m, &params).await,
        m if winmgr::handles(m) => winmgr::command(caller, m, &params).await,
        m if dialogs::handles(m) => dialogs::command(caller, m, &params).await,
        m if inspect::handles(m) => inspect::command(caller, m, &params).await,
        m if misc::handles(m) => misc::command(caller, m, &params).await,
        m if lifecycle::handles(m) => lifecycle::command(caller, m, &params).await,
        m if sdk::handles(m) => sdk::command(caller, m, &params).await,
        "tabs.activate" | "tab.activate" => {
            // A frozen or discarded tab wakes up when it is shown; a tab in
            // another window comes to the top of that window.
            if let Some(tab) = params["tabId"].as_str().map(str::to_string) {
                winmgr::shown(&tab).await;
                if winmgr::activate_detached(&tab) {
                    sessions::authorize_tab(caller, &tab)?;
                    return Ok(
                        json!({"accepted": true, "tabId": tab, "windowId": winmgr::window_of(&tab)}),
                    );
                }
            }
            run_core(caller, method, params)
        }
        "tabs.create" | "tab.create" if profiles::wants(caller, &params) => {
            lifecycle::make_room(caller).await;
            profiles::create_tab(caller, &params).await
        }
        "tabs.create" | "tab.create" => {
            lifecycle::make_room(caller).await;
            run_core(caller, method, params)
        }
        m if shell::handles(m) => shell::command(caller, m, &params).await,
        m if skills::handles(m) => skills::command(caller, m, &params).await,
        // --- sessions, events, operator control ---------------------------
        "session.info" => Ok(json!({
            "session": caller.session,
            "instanceId": state::state().instance_id,
            "protocol": agentgo_protocol::PROTOCOL_VERSION,
            "profile": state::profile_info(),
            "controlMode": state::control_mode(),
        })),
        "events.subscribe" | "events.unsubscribe" => {
            sessions::subscribe(&caller.session, method == "events.subscribe");
            Ok(json!({"subscribed": method == "events.subscribe"}))
        }
        "agent.status" => Ok(sessions::status()),
        "agent.connect" | "agent.disconnect" => {
            // The name defaults to the client that called (its session.hello name).
            let client = sessions::client_of(&caller.session);
            let name = params["name"]
                .as_str()
                .map(str::to_string)
                .unwrap_or(client);
            if method == "agent.connect" {
                sessions::connect(&name, None);
                Ok(json!({"agent": name, "connected": true}))
            } else {
                let was = sessions::disconnect(&name);
                Ok(json!({"agent": name, "connected": false, "wasConnected": was}))
            }
        }
        "accounts.list" => Ok(accounts::list()),
        "credentials.list" => accounts::credentials_list(),
        "credentials.offer_accept" | "credentials.offer_dismiss" => {
            shell_only(caller, method)?;
            accounts::offer_decide(&params, method == "credentials.offer_accept")
        }
        "credentials.save" | "credentials.delete" => {
            shell_only(caller, method)?;
            if method == "credentials.save" {
                accounts::credentials_save(&params)
            } else {
                accounts::credentials_delete(&params)
            }
        }
        "credentials.fill" => {
            let tab = page::tab_of(caller, &params)?;
            profiles::vault_allowed(&tab)?;
            sessions::check_handoff(caller, &tab)?;
            accounts::credentials_fill(&tab).await
        }
        "connectors.list" => Ok(accounts::connectors_list()),
        "connectors.call" => accounts::connectors_call(&params).await,
        "accounts.add" | "accounts.link" | "accounts.unlink" | "accounts.connect"
        | "accounts.block" | "accounts.unblock" => {
            shell_only(caller, method)?;
            match method {
                "accounts.block" => accounts::set_blocked(&params, true),
                "accounts.unblock" => accounts::set_blocked(&params, false),
                "accounts.connect" => accounts::connect(&params),
                "accounts.add" => accounts::add(&params),
                "accounts.link" => accounts::link(&params),
                _ => accounts::unlink(&params),
            }
        }
        "agent.pause" | "agent.resume" | "agent.stop" => {
            shell_only(caller, method)?;
            sessions::set_control(match method {
                "agent.pause" => Control::Paused,
                "agent.stop" => Control::Stopped,
                _ => Control::Running,
            });
            if method != "agent.resume" {
                state::agent_lock_release();
            }
            Ok(sessions::status())
        }
        "agent.approval.respond" => {
            shell_only(caller, method)?;
            let id = params["approvalId"]
                .as_str()
                .ok_or_else(|| Fail::params("approvalId is required"))?;
            sessions::decide_approval(id, params["approve"] == true)?;
            Ok(sessions::status())
        }
        "approval.wait" => {
            let id = params["approvalId"]
                .as_str()
                .ok_or_else(|| Fail::params("approvalId is required"))?
                .to_string();
            let timeout = params["timeout"]
                .as_u64()
                .unwrap_or(120_000)
                .clamp(100, 300_000);
            let started = std::time::Instant::now();
            loop {
                match sessions::approval_status(caller, &id) {
                    Some("pending") if (started.elapsed().as_millis() as u64) < timeout => {
                        exec::sleep(150).await
                    }
                    Some(status) => return Ok(json!({"approvalId": id, "status": status})),
                    None => return Err(Fail::params(format!("unknown approval {id}"))),
                }
            }
        }
        "handoff.wait" => {
            let tab = page::tab_of(caller, &params)?;
            let timeout = params["timeout"]
                .as_u64()
                .unwrap_or(120_000)
                .clamp(100, 600_000);
            let started = std::time::Instant::now();
            while sessions::handoff(&tab).is_some()
                && (started.elapsed().as_millis() as u64) < timeout
            {
                exec::sleep(200).await;
            }
            Ok(json!({"tabId": tab, "resolved": sessions::handoff(&tab).is_none()}))
        }
        "handoff.resolve" => {
            shell_only(caller, method)?;
            let tab = page::tab_of(caller, &params)?;
            // Resume only after the challenge is really gone.
            let signals = cdp::agent_eval(&tab, "__ag.signals()", 6_000)
                .await
                .unwrap_or(Value::Null);
            if signals["captcha"] == true {
                return Err(Fail::new(
                    ErrorCode::HumanHandoff,
                    "the verification challenge is still on the page",
                ));
            }
            sessions::end_handoff(&tab);
            Ok(sessions::status())
        }
        "handoff.view" => {
            shell_only(caller, method)?;
            let tab = page::tab_of(caller, &params)?;
            state::activate_tab(&tab)
                .map_err(|message| Fail::new(ErrorCode::TargetStale, message))?;
            Ok(json!({"activeTabId": tab}))
        }

        // --- observation --------------------------------------------------
        "observe.page" | "observe.interactive" | "observe.outline" => {
            page::observe_page(caller, &params).await
        }
        "observe.text"
        | "observe.content"
        | "observe.links"
        | "observe.tables"
        | "observe.forms"
        | "observe.page_metadata"
        | "form.discover" => page::observe_view(caller, method, &params).await,
        "observe.screenshot" | "observe.full_page_screenshot" | "observe.crop" | "capture.page" => {
            page::screenshot(caller, method, &params).await
        }
        "observe.dialogs" | "dialog.list" => page::dialog(caller, "dialog.list", &params).await,
        "state.diff" | "state.delta_after_action" => page::state_diff(caller, &params).await,
        method if method.starts_with("wait.") => page::wait(caller, method, &params).await,

        // --- actions ------------------------------------------------------
        "act.click"
        | "act.double_click"
        | "act.right_click"
        | "act.hover"
        | "pointer.click"
        | "pointer.double_click"
        | "pointer.triple_click"
        | "pointer.right_click"
        | "pointer.middle_click"
        | "pointer.hover"
        | "pointer.move" => {
            let mut out = page::act_click(caller, method, &params).await?;
            out["accepted"] = json!(true);
            Ok(out)
        }
        "act.drag" => page::act_drag(caller, &params).await,
        method if method.starts_with("clipboard.") => {
            page::clipboard(caller, method, &params).await
        }
        "page.find" => page::find(caller, &params).await,
        "page.zoom" => page::zoom(caller, &params),
        "act.fill" | "edit.fill" => page::act_fill(caller, &params).await,
        "form.fill" | "form.fill_batch" => page::form_fill(caller, &params).await,
        "act.type" | "keyboard.type" | "keyboard.insert_text" => {
            let mut out = page::act_type(caller, &params).await?;
            out["accepted"] = json!(true);
            Ok(out)
        }
        "act.press" | "keyboard.press" | "keyboard.shortcut" => {
            let mut out = page::act_press(caller, &params).await?;
            out["accepted"] = json!(true);
            Ok(out)
        }
        "act.check" | "act.uncheck" | "act.toggle" => {
            page::act_check(caller, method, &params).await
        }
        "act.select_option" => page::act_select(caller, &params).await,
        "act.scroll" | "act.scroll_page" | "pointer.scroll" | "pointer.scroll_into_view" => {
            let mut out = page::act_scroll(caller, &params).await?;
            out["accepted"] = json!(true);
            Ok(out)
        }
        "act.upload" | "file.upload" => page::act_upload(caller, &params).await,
        "dialog.accept" | "dialog.dismiss" | "dialog.confirm" | "dialog.prompt"
        | "dialog.alert" | "dialog.close" => page::dialog(caller, method, &params).await,
        "runtime.evaluate" => page::evaluate(caller, &params).await,
        "print.to_pdf" | "page.save_snapshot" => page::export(caller, method, &params).await,
        "network.observe_requests"
        | "network.requests"
        | "network.failures"
        | "network.observe_responses"
        | "runtime.console" => page::telemetry(caller, method, &params).await,
        "storage.cookies_list"
        | "storage.cookie_get"
        | "storage.cookie_set"
        | "storage.cookie_delete" => page::cookies(caller, method, &params).await,

        // --- navigation and workflows -------------------------------------
        "navigate.url" | "navigate.open" | "navigate.search"
            if params.get("observe").is_some() || method != "navigate.search" =>
        {
            navigate_open(caller, &params).await
        }
        "download.start" => {
            // CEF gives no synchronous result; confirm the transfer really
            // started instead of reporting an unverified "accepted".
            let before = state::downloads_snapshot().len();
            let mut out = run_core(caller, method, params)?;
            for _ in 0..100 {
                let now = state::downloads_snapshot();
                if now.len() > before {
                    out["id"] = now[0]["id"].clone();
                    out["path"] = now[0]["path"].clone();
                    return Ok(out);
                }
                exec::sleep(50).await;
            }
            Err(Fail::internal(
                "the browser did not start the download within 5 s (a blank tab cannot download; load a page first)",
            ))
        }
        "workflow.batch" => batch(caller, &params).await,
        "workflow.search_extract" => search_extract(caller, &params).await,
        "tab.reopen_closed" => {
            let url = CLOSED
                .with(|closed| closed.borrow_mut().pop())
                .ok_or_else(|| Fail::new(ErrorCode::TargetStale, "no closed tab to reopen"))?;
            Box::pin(run(caller, "tabs.create", json!({"url": url}))).await
        }
        "form.submit" => {
            // The submit button is clicked like any other: the approval
            // gate for final submissions and purchases applies.
            let tab = page::tab_of(caller, &params)?;
            let found = page::agent(
                &tab,
                "tool",
                json!(["form.submit_target", params.clone()]),
                8_000,
            )
            .await?;
            if let Some(fail) = page::agent_error(&found) {
                return Err(fail);
            }
            let mut click = json!({"tabId": tab, "ref": found["ref"]});
            if let Some(approval) = params.get("approvalId") {
                click["approvalId"] = approval.clone();
            }
            let mut out = Box::pin(run(caller, "act.click", click)).await?;
            out["submitted"] = found["line"].clone();
            Ok(out)
        }
        "file.path_validate" | "file.metadata" | "file.hash" | "file.read_text" => {
            file_command(method, &params)
        }
        "network.intercept"
        | "network.fulfill"
        | "network.modify_response"
        | "network.emulate_errors"
        | "network.continue"
        | "network.clear_interceptors"
        | "network.request_body"
        | "network.response_body"
        | "network.download_response"
        | "network.headers"
        | "network.websocket_list"
        | "network.websocket_messages"
        | "network.sse_events"
        | "network.har_stop"
        | "network.replay" => super::netctl::command(caller, method, &params).await,
        m if m.starts_with("metrics.") => suite::metrics(&caller.session, method, &params),
        "benchmark.run" | "benchmark.custom_suite" => benchmark_run(caller, &params).await,
        m if m.starts_with("benchmark.") => suite::benchmark(&caller.session, method, &params),
        m if m.starts_with("memory.") => suite::memory(method, &params),
        m if m.starts_with("research_cache.") => suite::research_cache(method, &params),
        "profile.session_list" | "profile.session_save" | "profile.session_resume" => {
            suite::saved_sessions(method, &params)
        }
        "profile.export_state"
        | "profile.import_state"
        | "profile.checkpoint"
        | "profile.restore_checkpoint" => {
            shell_only(caller, method)?;
            suite::profile_state(method, &params)
        }
        "policy.domain_blocklist" | "policy.domain_allowlist" => {
            if params.get("add").is_some() || params.get("remove").is_some() {
                shell_only(caller, method)?;
            }
            suite::domain_list(method, &params)
        }
        "policy.action_classify" | "policy.preview_side_effect" => {
            let target = params["method"]
                .as_str()
                .ok_or_else(|| Fail::params("params.method: the command to classify"))?;
            let registry = agentgo_shell_bridge::ShellMethodRegistry::new();
            let known = registry.lookup(target);
            let risk = known.map(|m| format!("{:?}", m.risk).to_ascii_lowercase());
            let approval = match risk.as_deref() {
                Some("money") => "always asks the person",
                Some("destructive") | Some("sensitive") if state::control_mode() == "approval" => {
                    "may ask the person (final submissions)"
                }
                _ => "runs without asking",
            };
            Ok(
                json!({"method": target, "known": known.is_some(), "risk": risk, "readOnly": read_only(target), "approval": approval, "operatorOnly": OPERATOR_SETTINGS.is_empty() && false}),
            )
        }
        "policy.approval_expire" => Ok(
            json!({"approvalsExpireAfterSeconds": 300, "singleUse": true, "boundTo": ["session", "tab", "origin", "target"]}),
        ),
        m if m.starts_with("artifact.")
            || matches!(
                m,
                "file.write_text"
                    | "file.copy"
                    | "file.move"
                    | "file.rename"
                    | "file.archive"
                    | "file.extract_archive"
            ) =>
        {
            suite::artifacts(&caller.session, method, &params)
        }
        "permission.list" => Ok(extras::list()),
        "extension.list" | "extension.permissions" | "extension.content_scripts" => {
            super::extensions::command(method, &params)
        }
        "extension.install" if params.get("store").is_some() => {
            shell_only(caller, method)?;
            super::extensions::install_from_store(params["store"].as_str().unwrap_or("")).await
        }
        "extension.install"
        | "extension.uninstall"
        | "extension.enable"
        | "extension.disable"
        | "extension.reload"
        | "extension.storage" => {
            shell_only(caller, method)?;
            super::extensions::command(method, &params)
        }
        "sync.status" => extras::sync(method, &params),
        "sync.start" | "sync.stop" => {
            shell_only(caller, method)?;
            extras::sync(method, &params)
        }
        "web_app.install" | "web_app.uninstall" => extras::web_app(caller, method, &params),
        "permission.grant" | "permission.deny" | "permission.reset" => {
            shell_only(caller, method)?;
            extras::change_permission(method, &params).await
        }
        "debug.devtools_open" | "debug.devtools_close" => {
            extras::devtools(caller, &params, method == "debug.devtools_open")
        }
        "print.to_printer" => extras::print_dialog(caller, &params),
        "download.show"
        | "download.open"
        | "navigate.external"
        | "native.open_with"
        | "native.external_app" => extras::outside(caller, method, &params),
        "window.native_handle" => Ok(json!({"hwnd": native::own_hwnd()})),
        _ if method.starts_with("native.")
            && !matches!(
                method,
                "native.open_with" | "native.external_app" | "native.screenshot"
            ) =>
        {
            native::command(caller, method, &params).await
        }
        "device.web_authn" => extras::web_authn(caller, &params).await,
        "observe.record_video" => extras::record_video(caller, &params).await,
        "pen.down" | "pen.move" | "pen.up" | "pen.pressure" => {
            page::pen(caller, method, &params).await
        }
        "touch.start"
        | "touch.move"
        | "touch.end"
        | "touch.cancel"
        | "touch.long_press"
        | "touch.swipe"
        | "touch.pinch"
        | "touch.rotate"
        | "pointer.down"
        | "pointer.up"
        | "pointer.drag_start"
        | "pointer.drag_move"
        | "pointer.drag_end"
        | "pointer.button_state"
        | "pointer.modifiers"
        | "keyboard.key_down"
        | "keyboard.key_up" => page::raw_input(caller, method, &params).await,
        "browser_shortcut.list" => Ok(json!({"shortcuts": [
            ["Ctrl+T", "new tab"], ["Ctrl+Shift+T", "reopen closed tab"], ["Ctrl+W", "close tab"],
            ["Ctrl+Tab / Ctrl+PgDn", "next tab"], ["Ctrl+Shift+Tab / Ctrl+PgUp", "previous tab"],
            ["Ctrl+1..8", "tab by position"], ["Ctrl+9", "last tab"], ["Ctrl+L / Alt+D / F6", "address bar"],
            ["Ctrl+R / F5", "reload"], ["Ctrl+Shift+R / Ctrl+F5", "reload without cache"],
            ["Alt+Left", "back"], ["Alt+Right", "forward"], ["Ctrl+H", "history"], ["Ctrl+J", "downloads"], ["Ctrl+K", "command palette"],
            ["Ctrl+D", "bookmark"], ["Ctrl+F / F3", "find"], ["Ctrl+P", "print"],
            ["Ctrl+Plus", "zoom in"], ["Ctrl+Minus", "zoom out"], ["Ctrl+0", "reset zoom"],
        ]})),
        "tab.duplicate" => {
            let tab = page::tab_of(caller, &params)?;
            let url = state::tab_url(&tab);
            Box::pin(run(caller, "tabs.create", json!({"url": url}))).await
        }
        "tab.close_others" | "tab.close_to_right" => {
            let tab = page::tab_of(caller, &params)?;
            let ids = state::content_tab_ids();
            let start = match method {
                "tab.close_to_right" => ids
                    .iter()
                    .position(|id| *id == tab)
                    .map_or(ids.len(), |at| at + 1),
                _ => 0,
            };
            let mut closed = Vec::new();
            // An agent closes only tabs it may see; the person's stay open.
            for id in ids.into_iter().skip(start) {
                if id != tab
                    && sessions::can_see_tab(caller, &id)
                    && Box::pin(run(caller, "tabs.close", json!({"tabId": id})))
                        .await
                        .is_ok()
                {
                    closed.push(id);
                }
            }
            Ok(json!({"kept": tab, "closed": closed}))
        }
        "tab.mute" => {
            let tab = page::tab_of(caller, &params)?;
            let muted = state::mute_tab(&tab, params["muted"] != false).map_err(Fail::internal)?;
            Ok(json!({"tabId": tab, "muted": muted}))
        }
        "browsing_data.clear" => {
            shell_only(caller, method)?;
            clear_browsing_data(&params).await
        }
        "workflow.record" => recorder::record(&caller.session, &params),
        "workflow.pause" | "workflow.resume" => {
            recorder::set_paused(&caller.session, method == "workflow.pause")
        }
        "workflow.disable" => recorder::disable(&params),
        "workflow.replay" => replay(caller, &params).await,
        "decision.status" => Ok(json!({
            "schema": decision::SCHEMA,
            // The person's switch, and whether a model is there to honor it.
            "enabled": state::settings_snapshot()["layaEnabled"] == true,
            "available": false,
            "model": null,
            "note": "Laya, the local choice model, is not bundled yet: its checkpoints still need fine-tuning. Until then your own model chooses; decision.record and decision.apply already use the format Laya will be given.",
        })),
        "decision.record" => {
            let tab = page::tab_of(caller, &params)?;
            let view = page::observe_page(caller, &json!({"tabId": tab, "max": params.get("max").and_then(Value::as_u64).unwrap_or(120)})).await?;
            let revision = page_revision(&tab, &view);
            let recent: Vec<Value> = sessions::status()["activity"]
                .as_array()
                .map(|items| items.iter().take(5).cloned().collect())
                .unwrap_or_default();
            decision::record(
                &caller.session,
                &tab,
                revision,
                &view,
                json!(recent),
                &params,
            )
            .map_err(Fail::params)
        }
        "decision.apply" => {
            let tab = page::tab_of(caller, &params)?;
            let view = page::observe_page(caller, &json!({"tabId": tab, "max": 120})).await?;
            let revision = page_revision(&tab, &view);
            match decision::resolve(&caller.session, &tab, &revision, &params) {
                Ok((step, step_params)) => {
                    let mut out = Box::pin(run(caller, &step, step_params)).await?;
                    out["ran"] = json!(step);
                    Ok(out)
                }
                // done and escalate end the chooser's turn; nothing runs.
                Err(control) if control.starts_with("control:") => {
                    Ok(json!({"ran": null, "control": control.trim_start_matches("control:")}))
                }
                Err(stale) if stale.starts_with("stale") => {
                    Err(Fail::new(ErrorCode::TargetStale, stale))
                }
                Err(other) => Err(Fail::params(other)),
            }
        }
        "workflow.collect" => collect(caller, &params).await,
        "workflow.fill_form" => page::form_fill(caller, &params).await,

        // --- browser product core (synchronous handlers) -------------------
        _ => run_core(caller, method, params),
    }
}

/// Tab/window/settings/download commands that complete synchronously.
fn run_core(caller: &Caller, method: &str, mut params: Value) -> Reply {
    let targets_tab = method.starts_with("tab.")
        || matches!(
            method,
            "tabs.navigate"
                | "tabs.status"
                | "tabs.close"
                | "tabs.activate"
                | "siteinfo.get"
                | "download.start"
        );
    if targets_tab
        && !matches!(
            method,
            "tabs.create" | "tab.create" | "tabs.list" | "tab.list"
        )
    {
        // Resolve the caller's own default tab and enforce its lease.
        let tab = page::tab_of(caller, &params)?;
        if let Some(map) = params.as_object_mut() {
            map.insert("tabId".into(), json!(tab));
        } else {
            params = json!({"tabId": tab});
        }
    }
    // Tab placement depends on who is asking. Any copy of the marker the
    // caller sent is dropped first, so only this code can set it. It is added
    // to these methods only: others pass their parameters on as data (for
    // example `settings.set` stores them).
    if matches!(
        method,
        "tabs.create" | "tab.create" | "tabs.activate" | "tab.activate"
    ) {
        if params.is_null() {
            params = json!({});
        }
        if let Some(map) = params.as_object_mut() {
            map.remove(state::AGENT_MARK);
            if !caller.is_shell() {
                map.insert(state::AGENT_MARK.into(), json!(true));
            }
        }
    }
    if !caller.is_shell() {
        match method {
            "settings.set" => {
                let patch = params.get("settings").unwrap_or(&params);
                if let Some(key) = patch.as_object().and_then(|map| {
                    map.keys()
                        .find(|key| OPERATOR_SETTINGS.contains(&key.as_str()))
                }) {
                    return Err(Fail::new(
                        ErrorCode::PolicyDenied,
                        format!("{key} can only be changed by the operator in AgentGo settings or at launch"),
                    ));
                }
            }
            "browser.shutdown" | "window.close" if !state::state().options.stdio => {
                return Err(Fail::new(
                    ErrorCode::PolicyDenied,
                    "this browser was opened by the operator; an attached agent cannot close it",
                ));
            }
            "profile.delete" | "history.clear" | "window.always_on_top" => {
                shell_only(caller, method)?
            }
            _ => {}
        }
    }
    let closing = matches!(method, "tabs.close" | "tab.close")
        .then(|| page::tab_of(caller, &params).ok())
        .flatten()
        .map(|tab| state::tab_url(&tab))
        .filter(|url| url.starts_with("http"));
    let mut out = legacy(method, params)?;
    if let Some(url) = closing {
        CLOSED.with(|closed| {
            let mut closed = closed.borrow_mut();
            closed.push(url);
            let excess = closed.len().saturating_sub(10);
            closed.drain(..excess);
        });
    }
    match method {
        "tabs.create" | "tab.create" => {
            if let Some(tab) = out["tabId"].as_str() {
                sessions::authorize_tab(caller, tab)?;
            }
        }
        "tabs.status" | "tab.status" if !caller.is_shell() => {
            if let Some(map) = out.as_object_mut() {
                map.retain(|_, value| !value.is_null() && *value != false);
            }
        }
        "tabs.list" | "tab.list" if !caller.is_shell() => {
            if let Some(tabs) = out["tabs"].as_array_mut() {
                tabs.retain(|tab| {
                    tab["tabId"]
                        .as_str()
                        .is_some_and(|id| sessions::can_see_tab(caller, id))
                });
                // Agents get the compact form: only fields that carry signal.
                for tab in tabs.iter_mut() {
                    if let Some(map) = tab.as_object_mut() {
                        map.retain(|key, value| {
                            matches!(key.as_str(), "tabId" | "url" | "title")
                                || (matches!(
                                    key.as_str(),
                                    "active" | "loading" | "crashed" | "handoff"
                                ) && !value.is_null()
                                    && *value != false)
                        });
                    }
                }
            }
        }
        _ => {}
    }
    Ok(out)
}

fn summarize(params: &Value) -> String {
    for key in ["url", "query", "ref", "key", "text", "tabId"] {
        match params.get(key) {
            // Never log typed text: it may be a credential.
            Some(_) if key == "text" => return "text (redacted)".into(),
            Some(Value::String(text)) => {
                return format!("{key}={}", agentgo_artifacts::redact_text(text))
            }
            Some(other) => return format!("{key}={other}"),
            None => {}
        }
    }
    String::new()
}

/// Agents may open local files only inside a folder the operator granted at
/// launch (`--upload-dir`); otherwise a page could be made to read any file.
fn check_agent_file_url(url: &str) -> Result<(), Fail> {
    let trimmed = url.trim_start();
    if !trimmed
        .get(..5)
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("file:"))
    {
        return Ok(());
    }
    let rest = trimmed[5..].trim_start_matches('/');
    let path = rest.split(['?', '#']).next().unwrap_or("");
    let decoded = percent_decode_path(path).replace('/', "\\");
    if state::upload_allowed(&decoded) {
        Ok(())
    } else {
        Err(Fail::new(
            ErrorCode::PolicyDenied,
            "agents can open file:// pages only inside a folder the operator granted with --upload-dir",
        ))
    }
}

fn percent_decode_path(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
            if let Ok(v) = u8::from_str_radix(hex, 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Executes one command for a caller, with idempotency and audit.
pub async fn dispatch(caller: &Caller, method: &str, mut params: Value) -> Reply {
    // An explicit null means "not given" for every command.
    if let Some(map) = params.as_object_mut() {
        map.retain(|_, value| !value.is_null());
    }
    let key = params
        .get("idempotencyKey")
        .and_then(Value::as_str)
        .filter(|_| !read_only(method))
        .map(|key| (caller.session.clone(), key.to_string()));
    if let Some(key) = &key {
        if let Some(reply) = IDEMPOTENT.with(|cache| cache.borrow().0.get(key).cloned()) {
            return reply;
        }
    }
    if !caller.is_shell() {
        if let Some(url) = params.get("url").and_then(Value::as_str) {
            check_agent_file_url(url)?;
        }
    }
    let summary = summarize(&params);
    let started = std::time::Instant::now();
    let steps = json!({"steps": params.get("steps")});
    let trace = std::env::var_os("AGENTGO_CDP_TRACE").is_some();
    if trace {
        eprintln!("CMD-START {} {method}", caller.session);
    }
    let recording = recorder::active(&caller.session).then(|| params.clone());
    // Lock the pages against the person's input while an agent acts.
    let locks = !caller.is_shell() && !read_only(method) && sessions::control() == Control::Running;
    if locks {
        state::agent_lock_begin();
    }
    let reply = run(caller, method, params).await;
    if locks {
        state::agent_lock_end();
    }
    suite::observe(
        &caller.session,
        method,
        read_only(method),
        started.elapsed().as_millis() as u64,
        &steps,
        &reply,
    );
    if let Some(params) = recording {
        recorder::note(&caller.session, method, &params, &reply);
    }
    if trace {
        eprintln!("CMD-END {} {method} ok={}", caller.session, reply.is_ok());
    }
    if !read_only(method) {
        sessions::record(caller, method, &summary, reply.is_ok());
    }
    if let Some(key) = key {
        IDEMPOTENT.with(|cache| {
            let (map, order) = &mut *cache.borrow_mut();
            if order.len() >= 256 {
                if let Some(oldest) = order.pop_front() {
                    map.remove(&oldest);
                }
            }
            order.push_back(key.clone());
            map.insert(key, reply.clone());
        });
    }
    reply
}

/// Encodes a reply as one protocol response line.
pub fn encode(id: u64, reply: Reply) -> String {
    let response = match reply {
        Ok(result) => Response::ok(id, result),
        Err(fail) => {
            let mut response = Response::err(id, fail.code, fail.message);
            if let Some(error) = response.error.as_mut() {
                error.data = fail.data;
            }
            response
        }
    };
    let mut line = serde_json::to_string(&response).unwrap_or_else(|_| {
        format!("{{\"v\":1,\"id\":{id},\"ok\":false,\"error\":{{\"code\":\"internal\",\"message\":\"encode failed\"}}}}")
    });
    line.push('\n');
    line
}

/// Accepts one request from an agent transport (UI thread) and answers it on
/// the session's outgoing channel when the handler completes.
pub fn handle_request(caller: Caller, request: Request) {
    exec::spawn(async move {
        // Requests may arrive while the profile is still opening.
        for _ in 0..600 {
            if state::ready() {
                break;
            }
            exec::sleep(25).await;
        }
        let reply = if request.v != agentgo_protocol::envelope::ENVELOPE_VERSION {
            Err(Fail::new(
                ErrorCode::InvalidEnvelope,
                format!("unsupported envelope version {}", request.v),
            ))
        } else {
            dispatch(&caller, &request.method, request.params).await
        };
        sessions::send_line(&caller.session, encode(request.id, reply));
    });
}
