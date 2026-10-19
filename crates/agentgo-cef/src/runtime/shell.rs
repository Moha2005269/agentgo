//! `shell.*`: the AgentGo window's own panels. Each command shows the person
//! the matching panel (the Agents panel's task, cost, artifacts, errors and
//! diff sections; settings sections; the address bar) and returns to the
//! caller what the panel shows, so an agent and the person see the same thing.

use agentgo_protocol::ErrorCode;
use serde_json::{json, Value};

use super::sessions::{self, Caller, Fail, Reply};
use super::{dispatch, page, skills, state, tasks};

fn operator(caller: &Caller, method: &str) -> Result<(), Fail> {
    if caller.is_shell() {
        Ok(())
    } else {
        Err(Fail::new(
            ErrorCode::PolicyDenied,
            format!("{method} is the person's own control"),
        ))
    }
}

fn show(tool: &str, section: &str, data: &Value) -> Result<(), Fail> {
    state::show_tool(tool, section, data).map_err(Fail::internal)
}

async fn call(caller: &Caller, method: &str, params: Value) -> Reply {
    Box::pin(dispatch::run(caller, method, params)).await
}

/// The task a shell command is about: the named one, else the caller's
/// latest (running first).
async fn task(caller: &Caller, params: &Value) -> Reply {
    if params.get("taskId").is_some() {
        return call(caller, "agent.monitor", params.clone()).await;
    }
    let list = call(caller, "agent.monitor", json!({})).await?;
    let tasks = list["tasks"].as_array().cloned().unwrap_or_default();
    let pick = tasks
        .iter()
        .rev()
        .find(|t| t["status"] == "running")
        .or_else(|| {
            tasks
                .iter()
                .max_by_key(|t| t["taskId"].as_str().map(str::to_string))
        })
        .cloned();
    match pick {
        Some(brief) => call(caller, "agent.monitor", json!({"taskId": brief["taskId"]})).await,
        None => Ok(json!({"taskId": null, "goal": null, "status": "none"})),
    }
}

/// What the address bar suggests for `text`: open tabs, bookmarks, history,
/// the address itself and a search, best first.
pub async fn suggestions(caller: &Caller, text: &str) -> Vec<Value> {
    let text = text.trim();
    if text.is_empty() {
        return Vec::new();
    }
    let needle = text.to_lowercase();
    let mut out: Vec<Value> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let looks_like_address = text.starts_with("http://")
        || text.starts_with("https://")
        || (!text.contains(' ') && text.contains('.'));
    if looks_like_address {
        let url = if text.contains("://") {
            text.to_string()
        } else {
            format!("https://{text}")
        };
        seen.insert(page_key(&url));
        out.push(json!({"kind": "address", "title": text, "url": url}));
    }
    if let Ok(tabs) = call(caller, "tabs.list", json!({})).await {
        for tab in tabs["tabs"].as_array().cloned().unwrap_or_default() {
            let url = tab["url"].as_str().unwrap_or("").to_string();
            let title = tab["title"].as_str().unwrap_or("").to_string();
            if url.starts_with("http")
                && format!("{title} {url}").to_lowercase().contains(&needle)
                && seen.insert(page_key(&url))
            {
                let title = readable_title(&title, &url);
                out.push(json!({"kind": "tab", "title": title, "url": url, "tabId": tab["tabId"]}));
            }
        }
    }
    for (method, kind) in [("bookmark.list", "bookmark"), ("history.list", "history")] {
        if let Ok(list) = call(caller, method, json!({"query": text, "limit": 40})).await {
            for item in list["items"].as_array().cloned().unwrap_or_default() {
                let url = item["url"].as_str().unwrap_or("").to_string();
                // One row per page: tracking parameters do not make a new one.
                if url.starts_with("http") && seen.insert(page_key(&url)) {
                    let title = item["title"].as_str().unwrap_or("");
                    out.push(
                        json!({"kind": kind, "title": readable_title(title, &url), "url": url}),
                    );
                }
            }
        }
    }
    out.truncate(6);
    let (engine, base) = state::search_engine();
    let query: String = text
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect();
    out.insert(usize::from(looks_like_address), json!({"kind": "search", "title": format!("Search {engine} for {text}"), "url": state::search_url(&base, &query)}));
    for (index, item) in out.iter_mut().enumerate() {
        item["index"] = json!(index);
    }
    out
}

/// A page's identity for suggestions: no fragment, no trailing slash and no
/// tracking or session parameters (Google's sei/ved/ei, utm_*, ...).
fn page_key(url: &str) -> String {
    const NOISE: [&str; 12] = [
        "sei", "ved", "ei", "oq", "gs_lcp", "sca_esv", "sxsrf", "source", "sclient", "uact",
        "gs_lp", "fbclid",
    ];
    let url = url.split('#').next().unwrap_or(url);
    let (base, query) = url.split_once('?').unwrap_or((url, ""));
    let kept: Vec<&str> = query
        .split('&')
        .filter(|pair| {
            let key = pair.split('=').next().unwrap_or("");
            !pair.is_empty() && !NOISE.contains(&key) && !key.starts_with("utm_")
        })
        .collect();
    let base = base.trim_end_matches('/').to_lowercase();
    let base = base
        .strip_prefix("http://")
        .or_else(|| base.strip_prefix("https://"))
        .unwrap_or(&base);
    let base = base.strip_prefix("www.").unwrap_or(base);
    if kept.is_empty() {
        base.to_string()
    } else {
        format!("{base}?{}", kept.join("&"))
    }
}

fn decode(text: &str) -> String {
    let bytes = text.replace('+', " ").into_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        match (
            bytes[i],
            bytes.get(i + 1).copied().and_then(hex),
            bytes.get(i + 2).copied().and_then(hex),
        ) {
            (b'%', Some(hi), Some(lo)) => {
                out.push((hi * 16 + lo) as u8);
                i += 3;
            }
            (b, ..) => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The page title when there is one; otherwise "query - Engine search" for a
/// search result page, or the bare host and path.
fn readable_title(title: &str, url: &str) -> String {
    let title = title.trim();
    if !title.is_empty() && title != url {
        return title.to_string();
    }
    let rest = url.split("://").nth(1).unwrap_or(url);
    let full_host = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    let host = full_host.strip_prefix("www.").unwrap_or(full_host);
    let query = url.split_once('?').map(|(_, q)| q).unwrap_or("");
    let search = query.split('&').find_map(|pair| pair.strip_prefix("q="));
    match search {
        Some(q) if !q.is_empty() => {
            let engine = host.split('.').next().unwrap_or(host);
            let mut engine = engine.to_string();
            if let Some(first) = engine.get_mut(0..1) {
                first.make_ascii_uppercase();
            }
            format!("{} - {engine} search", decode(q))
        }
        _ => {
            let path = rest[full_host.len()..]
                .split(['?', '#'])
                .next()
                .unwrap_or("");
            format!("{host}{}", path.trim_end_matches('/'))
        }
    }
}

#[cfg(test)]
mod suggestion_tests {
    use super::{page_key, readable_title};

    #[test]
    fn tracking_parameters_do_not_make_a_new_page() {
        assert_eq!(
            page_key("https://www.google.com/search?q=google&sei=w8ygasO4"),
            page_key("https://www.google.com/search?q=google")
        );
        assert_ne!(
            page_key("https://www.google.com/search?q=a"),
            page_key("https://www.google.com/search?q=b")
        );
        assert_eq!(
            page_key("https://example.com/"),
            page_key("http://www.example.com#top")
        );
    }

    #[test]
    fn titles_are_readable() {
        assert_eq!(
            readable_title(
                "",
                "https://www.google.com/search?q=gemini%20are%20u%20here%3F&sei=x"
            ),
            "gemini are u here? - Google search"
        );
        assert_eq!(
            readable_title("", "https://duckduckgo.com/?q=agent+go"),
            "agent go - Duckduckgo search"
        );
        assert_eq!(
            readable_title("", "https://www.python.org/downloads/"),
            "python.org/downloads"
        );
        assert_eq!(
            readable_title("Python", "https://www.python.org/"),
            "Python"
        );
    }
}

thread_local! {
    static TYPED: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
}

pub fn handles(method: &str) -> bool {
    method.starts_with("omnibox.")
        || matches!(
            method,
            "shell.agent_goal"
                | "shell.agent_step"
                | "shell.agent_edit_plan"
                | "shell.agent_handoff"
                | "shell.agent_cost"
                | "shell.agent_artifacts"
                | "shell.agent_error"
                | "shell.agent_retry"
                | "shell.agent_replan"
                | "shell.command_bar"
                | "shell.quick_actions"
                | "shell.sidebar"
                | "shell.permission_center"
                | "shell.settings"
                | "shell.policy_editor"
                | "shell.diff_view"
                | "shell.target_highlight"
                | "shell.action_preview"
                | "shell.artifact_preview"
                | "shell.workflow_editor"
                | "shell.skill_browser"
        )
}

pub async fn command(caller: &Caller, method: &str, params: &Value) -> Reply {
    match method {
        "omnibox.focus" | "omnibox.clear" | "omnibox.type" => {
            let text = if method == "omnibox.type" {
                params["text"]
                    .as_str()
                    .ok_or_else(|| Fail::params("text is required"))?
            } else {
                ""
            };
            TYPED.with(|t| *t.borrow_mut() = text.to_string());
            let script = format!(
                "window.__agentgoShortcut && window.__agentgoShortcut('focus_address'); window.__agentgoOmnibox && window.__agentgoOmnibox({});",
                json!({"text": text, "show": method == "omnibox.type"})
            );
            let focused = state::chrome_script(&script);
            let items = if method == "omnibox.type" {
                suggestions(caller, text).await
            } else {
                Vec::new()
            };
            Ok(json!({"focused": focused, "text": text, "suggestions": items}))
        }
        "omnibox.suggestions" => {
            let text = params["text"]
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| TYPED.with(|t| t.borrow().clone()));
            Ok(json!({"text": text, "suggestions": suggestions(caller, &text).await}))
        }
        "omnibox.select_suggestion" => {
            let text = params["text"]
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| TYPED.with(|t| t.borrow().clone()));
            let items = suggestions(caller, &text).await;
            let index = params["index"].as_u64().unwrap_or(0) as usize;
            let item = items.get(index).cloned().ok_or_else(|| {
                Fail::params(format!(
                    "there are {} suggestions for {text:?}",
                    items.len()
                ))
            })?;
            state::chrome_script("window.__agentgoOmnibox && window.__agentgoOmnibox({text: '', show: false, blur: true});");
            if let (Some(tab), true) = (item["tabId"].as_str(), item["kind"] == "tab") {
                let out = call(caller, "tabs.activate", json!({"tabId": tab})).await?;
                return Ok(json!({"selected": item, "result": out}));
            }
            // Agents open it in their own tab; the person in the visible one.
            let mut open = json!({"url": item["url"]});
            if caller.is_shell() {
                open["tabId"] = json!(state::active_tab_id());
            } else if let Some(tab) = params.get("tabId") {
                open["tabId"] = tab.clone();
            }
            let out = call(caller, "navigate.url", open).await?;
            Ok(json!({"selected": item, "url": out["url"], "tabId": out["tabId"]}))
        }
        "shell.agent_goal" | "shell.agent_step" | "shell.agent_cost" => {
            let task = task(caller, params).await?;
            show("agent", "task", &json!({"taskId": task["taskId"]}))?;
            let cursor = task["cursor"].as_u64().unwrap_or(0) as usize;
            Ok(match method {
                "shell.agent_goal" => {
                    json!({"taskId": task["taskId"], "goal": task["goal"], "status": task["status"]})
                }
                "shell.agent_step" => json!({
                    "taskId": task["taskId"],
                    "step": cursor,
                    "of": task["steps"],
                    "current": task["plan"].get(cursor),
                    "status": task["status"],
                }),
                _ => {
                    let tokens = call(caller, "benchmark.token_report", json!({}))
                        .await
                        .unwrap_or(Value::Null);
                    json!({"taskId": task["taskId"], "used": task["used"], "budget": task["budget"], "tokens": tokens})
                }
            })
        }
        "shell.agent_error" => {
            let list = call(caller, "agent.monitor", json!({})).await?;
            let errors: Vec<Value> = list["tasks"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter(|t| t["error"].is_object())
                .map(|t| {
                    let error = &t["error"];
                    json!({
                        "taskId": t["taskId"],
                        "goal": t["goal"],
                        "error": error,
                        "recovery": tasks::classify(error["code"].as_str().unwrap_or(""), error["message"].as_str().unwrap_or("")),
                    })
                })
                .collect();
            show("agent", "task", &json!({}))?;
            Ok(json!({"errors": errors}))
        }
        "shell.agent_edit_plan" => {
            operator(caller, method)?;
            let task = call(caller, "agent.monitor", params.clone()).await?;
            let cursor = task["cursor"].as_u64().unwrap_or(0) as usize;
            let remove: Vec<usize> = params["remove"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .iter()
                .filter_map(|i| i.as_u64().map(|i| i as usize))
                .collect();
            let steps: Vec<Value> = match params.get("steps") {
                Some(steps) => steps.as_array().cloned().unwrap_or_default(),
                None => task["plan"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .enumerate()
                    .skip(cursor)
                    .filter(|(index, _)| !remove.contains(index))
                    .map(|(_, step)| json!({"method": step["method"], "params": step["params"]}))
                    .collect(),
            };
            let out = call(
                caller,
                "agent.replan",
                json!({"taskId": task["taskId"], "steps": steps}),
            )
            .await?;
            show("agent", "task", &json!({"taskId": task["taskId"]}))?;
            Ok(out)
        }
        "shell.agent_retry" | "shell.agent_replan" => {
            operator(caller, method)?;
            let task = call(caller, "agent.monitor", params.clone()).await?;
            let id = task["taskId"].clone();
            let owner = tasks::owner(id.as_str().unwrap_or("")).unwrap_or_default();
            if method == "shell.agent_replan" {
                // The person asks the agent to plan again; the agent reads the note.
                let note = params["note"]
                    .as_str()
                    .unwrap_or("The person asked for a new plan.")
                    .to_string();
                tasks::request_replan(id.as_str().unwrap_or(""), &note);
                sessions::emit(
                    "task.replan_requested",
                    json!({"taskId": id, "note": note, "session": owner}),
                );
                show("agent", "task", &json!({"taskId": id}))?;
                return Ok(json!({"taskId": id, "status": "replan", "note": note}));
            }
            // The retry runs as the agent that owns the task, with its gates.
            let agent = Caller { session: owner };
            let retry = json!({"taskId": id});
            super::exec::spawn(async move {
                let _ = Box::pin(dispatch::run(&agent, "agent.retry", retry)).await;
            });
            show("agent", "task", &json!({"taskId": id}))?;
            Ok(json!({"taskId": id, "retrying": true}))
        }
        "shell.agent_handoff" => {
            let status = sessions::status();
            show("agent", "handoff", &json!({}))?;
            Ok(json!({"handoffs": status["handoffs"], "approvals": status["approvals"]}))
        }
        "shell.agent_artifacts" => {
            let list = call(
                caller,
                "artifact.list",
                json!({"max": params["max"].as_u64().unwrap_or(20)}),
            )
            .await?;
            show("agent", "artifacts", &list)?;
            Ok(list)
        }
        "shell.diff_view" => {
            let diff = call(caller, "state.diff", params.clone()).await?;
            show("agent", "diff", &diff)?;
            Ok(diff)
        }
        "shell.command_bar" => {
            let text = params["text"].as_str().unwrap_or("");
            let script = format!(
                "window.__agentgoShortcut && window.__agentgoShortcut('focus_address'); {}",
                if text.is_empty() {
                    String::new()
                } else {
                    format!(
                        "document.querySelector('#address').value = {};",
                        json!(text)
                    )
                }
            );
            let focused = state::chrome_script(&script);
            Ok(json!({"focused": focused, "text": text}))
        }
        "shell.quick_actions" => {
            let tab = page::tab_of(caller, params).ok();
            let url = tab.as_deref().map(state::tab_url).unwrap_or_default();
            show("menu", "", &json!({}))?;
            Ok(json!({"url": url, "actions": [
                {"label": "Back", "method": "tab.back"},
                {"label": "Reload", "method": "tab.reload"},
                {"label": "Find in page", "method": "page.find"},
                {"label": "Bookmark this page", "method": "bookmark.add"},
                {"label": "Screenshot", "method": "observe.screenshot"},
                {"label": "Save as PDF", "method": "print.to_pdf"},
                {"label": "Zoom", "method": "page.zoom"},
                {"label": "Copy the address", "method": "clipboard.write", "params": {"text": url}},
            ]}))
        }
        "shell.sidebar" => {
            let panel = params["panel"].as_str().unwrap_or("agent");
            if !matches!(
                panel,
                "agent" | "history" | "downloads" | "bookmarks" | "accounts" | "settings"
            ) {
                return Err(Fail::params(
                    "panel: agent, history, downloads, bookmarks, accounts or settings",
                ));
            }
            show(panel, params["section"].as_str().unwrap_or(""), &json!({}))?;
            Ok(json!({"open": panel}))
        }
        "shell.settings" | "shell.permission_center" | "shell.policy_editor" => {
            let (section, out) = match method {
                "shell.permission_center" => {
                    ("privacy", call(caller, "permission.list", json!({})).await?)
                }
                "shell.policy_editor" => {
                    let settings = call(caller, "settings.get", json!({})).await?;
                    let allow = call(caller, "policy.domain_allowlist", json!({})).await?;
                    let block = call(caller, "policy.domain_blocklist", json!({})).await?;
                    (
                        "agent",
                        json!({
                            "controlMode": settings["controlMode"],
                            "nativeControl": settings["nativeControl"],
                            "allowEvaluate": settings["allowEvaluate"],
                            "downloadsEnabled": settings["downloadsEnabled"],
                            "allowlist": allow["domains"],
                            "blocklist": block["domains"],
                        }),
                    )
                }
                _ => (
                    params["section"].as_str().unwrap_or(""),
                    call(caller, "settings.get", json!({})).await?,
                ),
            };
            show("settings", section, &json!({}))?;
            Ok(out)
        }
        "shell.target_highlight" => {
            let mut target = params.clone();
            tasks::resolve(caller, &mut target).await?;
            let tab = page::tab_of(caller, &target)?;
            let id = target["ref"].clone();
            if id.is_null() {
                return Err(Fail::params("name the element: ref, or find {by, ...}"));
            }
            let shown = page::agent(&tab, "pointAt", json!([id, "move"]), 2_000).await?;
            // pointAt answers {shown: false} when the element is gone, else the glide time.
            Ok(json!({"ref": id, "shown": shown["shown"] != false, "tabId": tab}))
        }
        "shell.action_preview" => {
            let step = json!([{"method": params["method"], "params": params.get("params").cloned().unwrap_or(json!({}))}]);
            let preview = call(caller, "agent.preview", json!({"steps": step})).await?;
            let first = &preview["plan"][0];
            let text = format!(
                "Agent will: {} ({})",
                first["description"]
                    .as_str()
                    .unwrap_or(first["method"].as_str().unwrap_or("")),
                first["risk"].as_str().unwrap_or("")
            );
            if let Ok(tab) = page::tab_of(caller, params) {
                let _ = page::agent(
                    &tab,
                    "banner",
                    json!([text.chars().take(120).collect::<String>()]),
                    1_500,
                )
                .await;
            }
            Ok(json!({"preview": first, "shown": text}))
        }
        "shell.artifact_preview" => {
            let meta = call(caller, "artifact.metadata", params.clone()).await?;
            let path = meta["path"].as_str().unwrap_or_default().replace('\\', "/");
            let url = format!("file:///{}", path.trim_start_matches('/'));
            let opened = call(caller, "tabs.create", json!({"url": url})).await?;
            Ok(json!({"path": meta["path"], "tabId": opened["tabId"], "url": url}))
        }
        "shell.workflow_editor" => {
            let list = skills::list()["workflows"].clone();
            show("agent", "workflows", &json!({"workflows": list}))?;
            Ok(json!({"workflows": list}))
        }
        "shell.skill_browser" => {
            let list = json!({"skills": skills::list()["skills"]});
            show("agent", "skills", &list)?;
            Ok(list)
        }
        _ => Err(Fail::internal(format!("{method} is not a shell command"))),
    }
}
