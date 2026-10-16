//! The smaller remaining tools of the register: input-method composition,
//! rich and image clipboard, dropping files on a page, download helpers,
//! credentials, the browser's own lists (bookmarks, history, downloads,
//! shortcuts), translation, user scripts, pagination and source comparison,
//! policy scopes, `api.act` and the BrowserGym adapter.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;

use agentgo_protocol::ErrorCode;
use serde_json::{json, Value};

use super::sessions::{self, Caller, Fail, Reply};
use super::{cdp, dispatch, exec, page, state, tasks};

thread_local! {
    /// session -> (html, plain text) written with clipboard.write_html.
    static HTML: RefCell<BTreeMap<String, (String, String)>> = const { RefCell::new(BTreeMap::new()) };
    /// session -> PNG bytes written with clipboard.write_image.
    static IMAGE: RefCell<BTreeMap<String, Vec<u8>>> = const { RefCell::new(BTreeMap::new()) };
    /// name -> pagination state of collect.paginate.
    static PAGES: RefCell<BTreeMap<String, Value>> = const { RefCell::new(BTreeMap::new()) };
}

async fn call(caller: &Caller, method: &str, params: Value) -> Reply {
    Box::pin(dispatch::run(caller, method, params)).await
}

fn operator(caller: &Caller, what: &str) -> Result<(), Fail> {
    if caller.is_shell() {
        Ok(())
    } else {
        Err(Fail::new(
            ErrorCode::PolicyDenied,
            format!("{what} is the person's decision"),
        ))
    }
}

/// The reference a call names (ref, or find {by, ...}).
async fn reference(caller: &Caller, tab: &str, params: &Value) -> Result<u64, Fail> {
    let mut target = params.clone();
    target["tabId"] = json!(tab);
    tasks::resolve(caller, &mut target).await?;
    target["ref"]
        .as_u64()
        .or_else(|| {
            target["ref"]
                .as_str()
                .and_then(|s| s.trim_matches(['[', ']']).parse().ok())
        })
        .ok_or_else(|| Fail::params("name the element: ref, or find {by, ...}"))
}

/// Focuses a field for typing with the caret at its end (what is there is
/// kept); payment fields need the person's approval, as with act.fill.
async fn focus_field(caller: &Caller, tab: &str, params: &Value) -> Result<(), Fail> {
    if params.get("ref").is_none() && params.get("find").is_none() {
        return Ok(());
    }
    let id = reference(caller, tab, params).await?;
    let object = cdp::agent_object(tab, &format!("__ag.element({id})"))
        .await
        .map_err(|e| Fail::new(ErrorCode::TargetStale, e))?;
    let out = cdp::call(tab, "Runtime.callFunctionOn", json!({
        "objectId": object,
        "returnByValue": true,
        "functionDeclaration": "function(){ this.focus(); if (typeof this.value === 'string' && this.setSelectionRange) { try { this.setSelectionRange(this.value.length, this.value.length); } catch (e) {} } else if (this.isContentEditable) { const r = document.createRange(); r.selectNodeContents(this); r.collapse(false); const s = getSelection(); s.removeAllRanges(); s.addRange(r); } const hint = [this.autocomplete, this.name, this.id].join(' ').toLowerCase(); return /cc-|card|cvc|cvv/.test(hint); }",
    })).await.map_err(Fail::internal)?;
    if out["result"]["value"] == true {
        let origin = super::dispatch::web_origin(&state::tab_url(tab)).unwrap_or_default();
        sessions::require_approval(
            caller,
            tab,
            &origin,
            &format!("field {id}"),
            "typing into a payment field",
            params["approvalId"].as_str(),
        )?;
    }
    Ok(())
}

async fn acting_tab(caller: &Caller, params: &Value) -> Result<String, Fail> {
    sessions::gate(caller).await?;
    let tab = page::tab_of(caller, params)?;
    sessions::check_handoff(caller, &tab)?;
    if cdp::dialog(&tab).is_some() {
        return Err(Fail::new(
            ErrorCode::TargetBlocked,
            "a JavaScript dialog is open; dialog.accept or dialog.dismiss first",
        ));
    }
    Ok(tab)
}

fn pending_file() -> PathBuf {
    state::data_dir().join("downloads-pending.json")
}

/// Downloads that were running when the last run ended (read once, before
/// this run writes its own).
fn previous_downloads() -> &'static Vec<Value> {
    static PREVIOUS: std::sync::OnceLock<Vec<Value>> = std::sync::OnceLock::new();
    PREVIOUS.get_or_init(|| {
        std::fs::read(pending_file())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    })
}

/// Called whenever a download changes: keeps the unfinished ones on disk.
pub fn note_downloads(all: &[Value]) {
    previous_downloads();
    let open: Vec<&Value> = all
        .iter()
        .filter(|d| d["inProgress"] == true || d["status"] == "interrupted")
        .collect();
    let _ = std::fs::write(
        pending_file(),
        serde_json::to_vec(&open).unwrap_or_default(),
    );
}

fn user_scripts_file() -> PathBuf {
    state::data_dir().join("user-scripts.json")
}

fn user_scripts() -> Vec<Value> {
    std::fs::read(user_scripts_file())
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn save_user_scripts(list: &[Value]) -> Result<(), Fail> {
    let _ = std::fs::create_dir_all(state::data_dir());
    std::fs::write(
        user_scripts_file(),
        serde_json::to_vec_pretty(list).unwrap_or_default(),
    )
    .map_err(|e| Fail::internal(e.to_string()))
}

/// Runs the person's enabled user scripts that match the loaded page, each
/// in its own isolated world (like extension content scripts).
pub async fn inject_user_scripts(tab: String) {
    let url = state::tab_url(&tab);
    let scripts: Vec<Value> = user_scripts()
        .into_iter()
        .filter(|s| {
            s["enabled"] != false
                && s["matches"].as_array().is_some_and(|m| {
                    m.iter()
                        .filter_map(Value::as_str)
                        .any(|p| super::extensions::matches(p, &url))
                })
        })
        .collect();
    if scripts.is_empty() {
        return;
    }
    let Ok(tree) = cdp::call(&tab, "Page.getFrameTree", json!({})).await else {
        return;
    };
    let Some(frame) = tree["frameTree"]["frame"]["id"]
        .as_str()
        .map(str::to_string)
    else {
        return;
    };
    for script in scripts {
        let name = script["name"].as_str().unwrap_or("script");
        let Ok(world) = cdp::call(
            &tab,
            "Page.createIsolatedWorld",
            json!({"frameId": frame, "worldName": format!("agentgo-user-{name}")}),
        )
        .await
        else {
            continue;
        };
        if let Some(context) = world["executionContextId"].as_i64() {
            let _ = cdp::call(
                &tab,
                "Runtime.evaluate",
                json!({"expression": script["code"], "contextId": context}),
            )
            .await;
            state::log_event(&format!("AG-RUNTIME user-script-ran {name} tab={tab}"));
        }
    }
}

/// Text with secrets and personal data blanked.
pub fn redact(text: &str) -> (String, usize) {
    let first = agentgo_artifacts::redact_text(text);
    let mut count = first.matches(agentgo_artifacts::redact::REDACTED).count();
    let mut out = String::with_capacity(first.len());
    let mut bearer = false;
    for word in first.split_inclusive(char::is_whitespace) {
        let core = word.trim_end();
        let digits = core.chars().filter(char::is_ascii_digit).count();
        let tokenish = core.len() >= 32
            && core
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_.=".contains(c))
            && core.chars().any(|c| c.is_ascii_digit());
        if bearer || (core.contains('@') && core.contains('.')) || digits >= 9 || tokenish {
            out.push_str(agentgo_artifacts::redact::REDACTED);
            out.push_str(&word[core.len()..]);
            count += 1;
        } else {
            out.push_str(word);
        }
        bearer = core.eq_ignore_ascii_case("bearer");
    }
    (out, count)
}

/// BrowserGym's action strings: click('12'), fill('12', 'text'), goto('url'),
/// press('12', 'Enter'), select_option('12', 'x'), hover('12'), scroll(0, 200),
/// go_back(), go_forward(), noop(), send_msg_to_user('...').
fn parse_gym(action: &str) -> Result<(String, Vec<String>), Fail> {
    let action = action.trim();
    let open = action
        .find('(')
        .ok_or_else(|| Fail::params("a BrowserGym action looks like click('12')"))?;
    let name = action[..open].trim().to_string();
    let inner = action[open + 1..]
        .trim_end()
        .strip_suffix(')')
        .ok_or_else(|| Fail::params("missing )"))?;
    let mut args = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for c in inner.chars() {
        match quote {
            Some(q) if escaped => {
                current.push(c);
                escaped = false;
                let _ = q;
            }
            Some(_) if c == '\\' => escaped = true,
            Some(q) if c == q => quote = None,
            Some(_) => current.push(c),
            None if c == '\'' || c == '"' => quote = Some(c),
            None if c == ',' => args.push(std::mem::take(&mut current).trim().to_string()),
            None => current.push(c),
        }
    }
    if !current.trim().is_empty() {
        args.push(current.trim().to_string());
    }
    Ok((name, args))
}

pub fn handles(method: &str) -> bool {
    matches!(
        method,
        "navigate.prerender_status"
            | "navigate.resume_after_interruption"
            | "keyboard.composition_start"
            | "keyboard.composition_update"
            | "keyboard.composition_end"
            | "edit.input_method"
            | "clipboard.write_html"
            | "clipboard.read_image"
            | "clipboard.write_image"
            | "act.drag_file"
            | "file.drag_to_upload"
            | "act.download"
            | "act.dispatch_input"
            | "download.resume_after_restart"
            | "download.deduplicate"
            | "download.quarantine"
            | "credential.revoke_session"
            | "credential.redact"
            | "network.cache_status"
            | "browser_ui.translate"
            | "bookmark.edit"
            | "bookmark.move"
            | "history.delete"
            | "download_manager.search"
            | "download_manager.remove"
            | "browser_shortcut.configure"
            | "script.user_list"
            | "script.user_install"
            | "script.user_remove"
            | "script.user_enable"
            | "script.user_disable"
            | "collect.paginate"
            | "collect.resume"
            | "research.compare_sources"
            | "policy.profile_scope"
            | "policy.file_scope"
            | "policy.network_scope"
            | "policy.permission_scope"
            | "handoff.cancel"
            | "api.act"
            | "benchmark.browsergym"
    )
}

pub async fn command(caller: &Caller, method: &str, params: &Value) -> Reply {
    match method {
        "navigate.prerender_status" => {
            let tab = page::tab_of(caller, params)?;
            let _ = cdp::call(&tab, "Preload.enable", json!({})).await;
            let now = cdp::agent_eval(&tab, "({prerendering: document.prerendering === true, activationStart: (performance.getEntriesByType('navigation')[0] || {}).activationStart || 0, rules: document.querySelectorAll('script[type=speculationrules]').length, prefetchLinks: document.querySelectorAll('link[rel=prefetch],link[rel=prerender]').length})", 4_000).await.map_err(Fail::internal)?;
            Ok(json!({
                "prerendered": now["activationStart"].as_f64().unwrap_or(0.0) > 0.0,
                "prerendering": now["prerendering"],
                "speculationRules": now["rules"],
                "prefetchLinks": now["prefetchLinks"],
                "events": cdp::events(&tab, &["prerender"], 50, false),
            }))
        }
        "navigate.resume_after_interruption" => {
            let tab = page::tab_of(caller, params)?;
            let url = state::tab_url(&tab);
            let failed = url
                .strip_prefix("agentgo://shell/error.html?")
                .and_then(|query| query.split('&').find_map(|pair| pair.strip_prefix("url=")))
                .map(|encoded| {
                    let bytes = encoded.as_bytes();
                    let mut out = Vec::new();
                    let mut i = 0;
                    while i < bytes.len() {
                        if bytes[i] == b'%' && i + 2 < bytes.len() {
                            if let Ok(b) = u8::from_str_radix(&encoded[i + 1..i + 3], 16) {
                                out.push(b);
                                i += 3;
                                continue;
                            }
                        }
                        out.push(if bytes[i] == b'+' { b' ' } else { bytes[i] });
                        i += 1;
                    }
                    String::from_utf8_lossy(&out).to_string()
                });
            match failed.or_else(|| params["url"].as_str().map(str::to_string)) {
                Some(target) => {
                    let out =
                        call(caller, "navigate.url", json!({"tabId": tab, "url": target})).await?;
                    Ok(json!({"resumed": true, "url": out["url"], "loaded": out["loaded"]}))
                }
                None if state::tab_is_loading(&tab) => {
                    let out = call(
                        caller,
                        "wait.load",
                        json!({"tabId": tab, "timeout": 20_000}),
                    )
                    .await?;
                    Ok(json!({"resumed": true, "waited": out}))
                }
                None => Ok(
                    json!({"resumed": false, "url": url, "reason": "the page was not interrupted"}),
                ),
            }
        }
        "keyboard.composition_start"
        | "keyboard.composition_update"
        | "keyboard.composition_end"
        | "edit.input_method" => {
            let tab = acting_tab(caller, params).await?;
            focus_field(caller, &tab, params).await?;
            let text = params["text"].as_str().unwrap_or("");
            match method {
                "keyboard.composition_end" => {
                    // Commits the text; with no text the composition is dropped.
                    if text.is_empty() {
                        cdp::input(
                            &tab,
                            "Input.imeSetComposition",
                            json!({"text": "", "selectionStart": 0, "selectionEnd": 0}),
                        )
                        .await
                        .map_err(Fail::internal)?;
                    } else {
                        cdp::input(&tab, "Input.insertText", json!({"text": text}))
                            .await
                            .map_err(Fail::internal)?;
                    }
                    Ok(json!({"committed": text}))
                }
                "edit.input_method" => {
                    // What an input method does: show each stage, then commit.
                    let stages: Vec<String> = match params["stages"].as_array() {
                        Some(list) => list
                            .iter()
                            .filter_map(|s| s.as_str().map(str::to_string))
                            .collect(),
                        None => {
                            let chars: Vec<char> =
                                params["reading"].as_str().unwrap_or(text).chars().collect();
                            (1..=chars.len())
                                .map(|n| chars[..n].iter().collect())
                                .collect()
                        }
                    };
                    for stage in &stages {
                        let len = stage.chars().count();
                        cdp::input(
                            &tab,
                            "Input.imeSetComposition",
                            json!({"text": stage, "selectionStart": len, "selectionEnd": len}),
                        )
                        .await
                        .map_err(Fail::internal)?;
                        exec::sleep(15).await;
                    }
                    cdp::input(&tab, "Input.insertText", json!({"text": text}))
                        .await
                        .map_err(Fail::internal)?;
                    let value = page::agent(&tab, "focusedEditable", json!([]), 4_000)
                        .await
                        .unwrap_or(Value::Null);
                    Ok(json!({"stages": stages.len(), "committed": text, "field": value}))
                }
                _ => {
                    let len = text.chars().count();
                    cdp::input(
                        &tab,
                        "Input.imeSetComposition",
                        json!({"text": text, "selectionStart": len, "selectionEnd": len}),
                    )
                    .await
                    .map_err(Fail::internal)?;
                    Ok(json!({"composing": text}))
                }
            }
        }
        "clipboard.write_html" => {
            let html = params["html"]
                .as_str()
                .ok_or_else(|| Fail::params("html is required"))?;
            if html.len() > 200_000 {
                return Err(Fail::params("the clipboard is limited to 200000 bytes"));
            }
            let mut text = String::new();
            let mut inside = false;
            for c in html.chars() {
                match c {
                    '<' => inside = true,
                    '>' => inside = false,
                    _ if !inside => text.push(c),
                    _ => {}
                }
            }
            let text = params["text"].as_str().map(str::to_string).unwrap_or(text);
            HTML.with(|h| {
                h.borrow_mut()
                    .insert(caller.session.clone(), (html.to_string(), text.clone()))
            });
            call(
                caller,
                "clipboard.write",
                json!({"text": if text.is_empty() { " ".to_string() } else { text.clone() }}),
            )
            .await?;
            if params["paste"] == true {
                let tab = acting_tab(caller, params).await?;
                focus_field(caller, &tab, params).await?;
                let pasted = cdp::agent_eval(&tab, &format!("(() => {{ const el = document.activeElement; if (!el || !el.isContentEditable) return {{error: 'blocked', detail: 'rich text goes into an editable area; plain text was put on the clipboard'}}; return {{pasted: document.execCommand('insertHTML', false, {})}}; }})()", json!(html)), 4_000).await.map_err(Fail::internal)?;
                if let Some(fail) = page::agent_error(&pasted) {
                    return Err(fail);
                }
                return Ok(
                    json!({"html": html.len(), "text": text.chars().count(), "pasted": pasted["pasted"]}),
                );
            }
            Ok(json!({"html": html.len(), "text": text.chars().count()}))
        }
        "clipboard.write_image" => {
            let bytes = match (params["path"].as_str(), params["base64"].as_str()) {
                (Some(path), _) => {
                    if !caller.is_shell()
                        && !state::upload_allowed(path)
                        && !path.contains("artifacts")
                    {
                        return Err(Fail::new(
                            ErrorCode::PolicyDenied,
                            format!("{path} is outside the upload folders and artifacts"),
                        ));
                    }
                    std::fs::read(path).map_err(|e| Fail::params(format!("{path}: {e}")))?
                }
                (None, Some(data)) => {
                    page::decode_base64(data.split_once(',').map_or(data, |(_, b)| b))
                }
                _ => return Err(Fail::params("path (a PNG file) or base64 is required")),
            };
            let (mime, _) = image_kind(&bytes)
                .ok_or_else(|| Fail::params("the image must be a PNG, JPEG, GIF or WebP"))?;
            IMAGE.with(|i| i.borrow_mut().insert(caller.session.clone(), bytes.clone()));
            if params["paste"] == true {
                // Pages read pasted images from the paste event's files.
                let tab = acting_tab(caller, params).await?;
                focus_field(caller, &tab, params).await?;
                let data = agentgo_artifacts::sha256_hex(&bytes);
                let b64: String = {
                    const T: &[u8] =
                        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
                    bytes
                        .chunks(3)
                        .flat_map(|c| {
                            let n = (c[0] as u32) << 16
                                | (*c.get(1).unwrap_or(&0) as u32) << 8
                                | *c.get(2).unwrap_or(&0) as u32;
                            (0..4).map(move |i| {
                                if i <= c.len() {
                                    T[(n >> (18 - 6 * i) & 63) as usize] as char
                                } else {
                                    '='
                                }
                            })
                        })
                        .collect()
                };
                let script = format!("(() => {{ const bin = atob('{b64}'); const a = new Uint8Array(bin.length); for (let i = 0; i < bin.length; i++) a[i] = bin.charCodeAt(i); const file = new File([a], 'image-{}', {{type: '{mime}'}}); const dt = new DataTransfer(); dt.items.add(file); const target = document.activeElement || document.body; return {{delivered: target.dispatchEvent(new ClipboardEvent('paste', {{clipboardData: dt, bubbles: true, cancelable: true}})) === false || true}}; }})()", &data[..8]);
                let out = cdp::agent_eval(&tab, &script, 6_000)
                    .await
                    .map_err(Fail::internal)?;
                return Ok(
                    json!({"bytes": bytes.len(), "pasted": out["delivered"], "note": "delivered as a paste event with the image as a file"}),
                );
            }
            Ok(json!({"bytes": bytes.len()}))
        }
        "clipboard.read_image" => {
            let bytes = IMAGE
                .with(|i| i.borrow().get(&caller.session).cloned())
                .ok_or_else(|| {
                    Fail::params("the agent clipboard holds no image; clipboard.write_image first")
                })?;
            let (_, extension) = image_kind(&bytes).unwrap_or(("image/png", "png"));
            let mut saved = page::save_artifact(caller, "clipboard", extension, &bytes)?;
            saved["bytes"] = json!(bytes.len());
            Ok(saved)
        }
        "act.drag_file" | "file.drag_to_upload" => {
            let tab = acting_tab(caller, params).await?;
            let paths: Vec<String> = match params.get("paths").or_else(|| params.get("path")) {
                Some(Value::String(p)) => vec![p.clone()],
                Some(Value::Array(list)) => list
                    .iter()
                    .filter_map(|p| p.as_str().map(str::to_string))
                    .collect(),
                _ => return Err(Fail::params("paths (files to drop) are required")),
            };
            for path in &paths {
                if !caller.is_shell() && !state::upload_allowed(path) {
                    return Err(Fail::new(ErrorCode::PolicyDenied, format!("{path} is outside the upload directories granted by the operator (launch with --upload-dir)")));
                }
                if !std::path::Path::new(path).is_file() {
                    return Err(Fail::params(format!("{path} is not a file")));
                }
            }
            let id = reference(caller, &tab, params).await?;
            let target = page::agent(&tab, "target", json!([id, {}]), 8_000).await?;
            if let Some(fail) = page::agent_error(&target) {
                return Err(fail);
            }
            let (x, y) = (
                target["x"].as_f64().unwrap_or(0.0),
                target["y"].as_f64().unwrap_or(0.0),
            );
            page::show(&tab, "pointTo", json!([x, y, "move"])).await;
            let data = json!({"items": [], "files": paths, "dragOperationsMask": 1});
            for kind in ["dragEnter", "dragOver", "drop"] {
                cdp::input(
                    &tab,
                    "Input.dispatchDragEvent",
                    json!({"type": kind, "x": x, "y": y, "data": data}),
                )
                .await
                .map_err(Fail::internal)?;
            }
            Ok(json!({"dropped": paths.len(), "on": target["line"], "x": x, "y": y}))
        }
        "act.download" => {
            if !state::downloads_enabled() {
                return Err(Fail::new(
                    ErrorCode::PolicyDenied,
                    "downloads are off; the person can turn on Allow downloads in Settings",
                ));
            }
            if let Some(url) = params["url"].as_str() {
                let known: Vec<Value> = state::downloads_snapshot()
                    .iter()
                    .map(|d| d["id"].clone())
                    .collect();
                let started = call(
                    caller,
                    "download.start",
                    json!({"url": url, "tabId": params.get("tabId")}),
                )
                .await?;
                let waited = call(
                    caller,
                    "wait.download",
                    json!({"id": started["id"], "known": known, "timeout": params["timeout"]}),
                )
                .await;
                return Ok(json!({"started": started, "finished": waited.ok()}));
            }
            let tab = page::tab_of(caller, params)?;
            let id = reference(caller, &tab, params).await?;
            let known: Vec<Value> = state::downloads_snapshot()
                .iter()
                .map(|d| d["id"].clone())
                .collect();
            let waiting = call(
                caller,
                "act.click",
                json!({"tabId": tab, "ref": id, "approvalId": params.get("approvalId")}),
            )
            .await?;
            let done = call(
                caller,
                "wait.download",
                json!({"timeout": params["timeout"].as_u64().unwrap_or(30_000), "known": known}),
            )
            .await?;
            Ok(
                json!({"clicked": waiting["target"], "download": done["download"], "complete": done["complete"]}),
            )
        }
        "act.dispatch_input" => {
            // Raw input goes through the gated commands, never around them.
            let kind = params["type"].as_str().unwrap_or("");
            let (target, mut routed) = match kind {
                "click" | "mousePressed" => (
                    "act.click",
                    json!({"x": params["x"], "y": params["y"], "button": params["button"]}),
                ),
                "doubleClick" => (
                    "act.double_click",
                    json!({"x": params["x"], "y": params["y"]}),
                ),
                "move" | "mouseMoved" => {
                    ("pointer.move", json!({"x": params["x"], "y": params["y"]}))
                }
                "wheel" | "mouseWheel" => (
                    "pointer.scroll",
                    json!({"x": params["x"], "y": params["y"], "deltaX": params["deltaX"], "deltaY": params["deltaY"]}),
                ),
                "key" | "keyDown" => (
                    "keyboard.press",
                    json!({"key": params["key"], "modifiers": params["modifiers"]}),
                ),
                "text" | "char" => ("keyboard.type", json!({"text": params["text"]})),
                "touch" => ("touch.tap", json!({"x": params["x"], "y": params["y"]})),
                _ => {
                    return Err(Fail::params(
                        "type: click, doubleClick, move, wheel, key, text or touch",
                    ))
                }
            };
            if let (Some(map), Some(tab)) = (routed.as_object_mut(), params.get("tabId")) {
                map.insert("tabId".into(), tab.clone());
            }
            if let (Some(map), Some(id)) = (routed.as_object_mut(), params.get("approvalId")) {
                map.insert("approvalId".into(), id.clone());
            }
            let out = call(caller, target, routed).await?;
            Ok(json!({"dispatched": kind, "as": target, "result": out}))
        }
        "download.resume_after_restart" => {
            let earlier: Vec<Value> = previous_downloads()
                .iter()
                .filter(|d| d["url"].as_str().is_some_and(|u| u.starts_with("http")))
                .cloned()
                .collect();
            if params["start"] != true {
                return Ok(
                    json!({"interrupted": earlier, "next": "download.resume_after_restart {start: true} downloads them again"}),
                );
            }
            if !state::downloads_enabled() {
                return Err(Fail::new(
                    ErrorCode::PolicyDenied,
                    "downloads are off; the person can turn on Allow downloads in Settings",
                ));
            }
            let mut started = Vec::new();
            for item in earlier {
                if let Ok(out) = call(caller, "download.start", json!({"url": item["url"]})).await {
                    started.push(json!({"url": item["url"], "id": out["id"]}));
                }
            }
            Ok(json!({"restarted": started}))
        }
        "download.deduplicate" => {
            let root = state::download_root();
            let mut seen: BTreeMap<String, String> = BTreeMap::new();
            let mut duplicates = Vec::new();
            for entry in std::fs::read_dir(&root).into_iter().flatten().flatten() {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }
                let hash = agentgo_artifacts::sha256_hex(&std::fs::read(&path).unwrap_or_default());
                let shown = path.display().to_string();
                match seen.get(&hash) {
                    Some(first) => {
                        duplicates.push(json!({"path": shown, "sameAs": first, "sha256": hash}))
                    }
                    None => {
                        seen.insert(hash, shown);
                    }
                }
            }
            if params["remove"] == true {
                operator(caller, "deleting downloaded files")?;
                for dup in &duplicates {
                    let _ = std::fs::remove_file(dup["path"].as_str().unwrap_or(""));
                }
            }
            Ok(
                json!({"duplicates": duplicates, "removed": params["remove"] == true, "folder": root.display().to_string()}),
            )
        }
        "download.quarantine" => {
            let path = params["path"]
                .as_str()
                .ok_or_else(|| Fail::params("path (a downloaded file) is required"))?;
            let file = std::path::Path::new(path)
                .canonicalize()
                .map_err(|_| Fail::params(format!("{path} does not exist")))?;
            let root = state::download_root().canonicalize().unwrap_or_default();
            if !file.starts_with(&root) {
                return Err(Fail::new(
                    ErrorCode::PolicyDenied,
                    "only files in the download folder",
                ));
            }
            // Windows' Mark of the Web: an alternate data stream on the file.
            let stream = format!("{}:Zone.Identifier", file.display());
            let existing = std::fs::read_to_string(&stream).ok();
            if existing.is_none() {
                let source = state::downloads_snapshot()
                    .into_iter()
                    .find(|d| {
                        d["path"].as_str().is_some_and(|p| {
                            std::path::Path::new(p).canonicalize().ok() == Some(file.clone())
                        })
                    })
                    .and_then(|d| d["url"].as_str().map(str::to_string))
                    .unwrap_or_default();
                std::fs::write(
                    &stream,
                    format!("[ZoneTransfer]\r\nZoneId=3\r\nHostUrl={source}\r\n"),
                )
                .map_err(|e| Fail::internal(format!("could not mark the file: {e}")))?;
            }
            let now = std::fs::read_to_string(&stream).unwrap_or_default();
            Ok(
                json!({"path": file.display().to_string(), "marked": now.contains("ZoneId=3"), "wasMarked": existing.is_some(), "zone": now.lines().find(|l| l.starts_with("ZoneId")).unwrap_or(""), "scan": state::download_scan_status(&file.to_string_lossy())}),
            )
        }
        "credential.revoke_session" => {
            let tab = page::tab_of(caller, params)?;
            let origin = params["origin"]
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| state::tab_url(&tab));
            let origin = super::dispatch::web_origin(&origin)
                .ok_or_else(|| Fail::params("origin must be an http(s) address"))?;
            if super::profiles::profile_of(&tab) == "main" && !caller.is_shell() {
                return Err(Fail::new(ErrorCode::PolicyDenied, "signing the person out of a site is the person's; agents may do it in side profiles"));
            }
            let cookies = cdp::call(
                &tab,
                "Network.getCookies",
                json!({"urls": [format!("{origin}/")]}),
            )
            .await
            .map_err(Fail::internal)?;
            let mut removed = 0;
            for cookie in cookies["cookies"].as_array().into_iter().flatten() {
                if cdp::call(&tab, "Network.deleteCookies", json!({"name": cookie["name"], "domain": cookie["domain"], "path": cookie["path"]})).await.is_ok() {
                    removed += 1;
                }
            }
            Ok(json!({"origin": origin, "cookiesRemoved": removed, "signedOut": removed > 0}))
        }
        "credential.redact" => {
            let text = params["text"]
                .as_str()
                .ok_or_else(|| Fail::params("text is required"))?;
            let (out, count) = redact(text);
            Ok(json!({"text": out, "redacted": count}))
        }
        "network.cache_status" => {
            let tab = page::tab_of(caller, params)?;
            let entries = cdp::agent_eval(&tab, "performance.getEntriesByType('resource').concat(performance.getEntriesByType('navigation')).slice(-300).map(e => ({url: e.name, cached: e.transferSize === 0 && e.decodedBodySize > 0, transfer: e.transferSize, size: e.decodedBodySize, type: e.initiatorType || 'navigation'}))", 4_000).await.map_err(Fail::internal)?;
            let list = entries.as_array().cloned().unwrap_or_default();
            let filtered: Vec<Value> = match params["url"].as_str() {
                Some(part) => list
                    .into_iter()
                    .filter(|e| e["url"].as_str().is_some_and(|u| u.contains(part)))
                    .collect(),
                None => list,
            };
            let cached = filtered.iter().filter(|e| e["cached"] == true).count();
            Ok(
                json!({"resources": filtered.len(), "fromCache": cached, "entries": filtered.into_iter().take(100).collect::<Vec<_>>()}),
            )
        }
        "browser_ui.translate" => {
            let tab = page::tab_of(caller, params)?;
            let url = state::tab_url(&tab);
            if !url.starts_with("http") {
                return Err(Fail::params("only web pages can be translated"));
            }
            let to: String = params["to"]
                .as_str()
                .unwrap_or("en")
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
                .take(10)
                .collect();
            let out = page::agent(
                &tab,
                "translatePage",
                json!({"to": to, "force": params["force"] == true, "restore": params["restore"] == true}),
                60_000,
            )
            .await?;
            Ok(
                json!({"from": url, "to": to, "result": out, "note": "translated in place via Google Translate"}),
            )
        }
        "bookmark.edit" => {
            let url = params["url"]
                .as_str()
                .ok_or_else(|| Fail::params("url (the bookmark) is required"))?;
            let new_url = params["newUrl"].as_str();
            if new_url.is_some_and(|u| !u.starts_with("http")) {
                return Err(Fail::params("only http(s) pages can be bookmarked"));
            }
            let changed = state::with_store(|store| {
                store.edit_bookmark(url, new_url, params["title"].as_str())
            })
            .map_err(Fail::internal)?;
            if !changed {
                return Err(Fail::params(format!("{url} is not bookmarked")));
            }
            state::notify_shell_state();
            Ok(json!({"url": new_url.unwrap_or(url), "title": params["title"], "edited": true}))
        }
        "bookmark.move" => {
            let url = params["url"]
                .as_str()
                .ok_or_else(|| Fail::params("url (the bookmark) is required"))?;
            let mut all: Vec<String> = state::with_store(|store| store.bookmarks(500))
                .map_err(Fail::internal)?
                .into_iter()
                .filter_map(|b| b["url"].as_str().map(str::to_string))
                .collect();
            let from = all
                .iter()
                .position(|u| u == url)
                .ok_or_else(|| Fail::params(format!("{url} is not bookmarked")))?;
            let item = all.remove(from);
            let to = (params["index"].as_u64().unwrap_or(0) as usize).min(all.len());
            all.insert(to, item);
            let base = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            state::with_store(|store| store.order_bookmarks(&all, base)).map_err(Fail::internal)?;
            Ok(json!({"url": url, "index": to, "order": all}))
        }
        "history.delete" => {
            operator(caller, "deleting history")?;
            let since = params["sinceMinutes"].as_u64().map(|m| {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_secs())
                    .saturating_sub(m * 60)
            });
            if params.get("url").is_none() && params.get("contains").is_none() && since.is_none() {
                return Err(Fail::params("name what to delete: url, contains or sinceMinutes (history.clear deletes everything)"));
            }
            let gone =
                state::delete_history(params["url"].as_str(), params["contains"].as_str(), since)
                    .map_err(Fail::internal)?;
            Ok(json!({"deleted": gone}))
        }
        "download_manager.search" => {
            let needle = params["text"].as_str().unwrap_or("").to_lowercase();
            let items: Vec<Value> = state::downloads_snapshot()
                .into_iter()
                .filter(|d| {
                    format!(
                        "{} {} {}",
                        d["suggestedName"].as_str().unwrap_or(""),
                        d["url"].as_str().unwrap_or(""),
                        d["status"].as_str().unwrap_or("")
                    )
                    .to_lowercase()
                    .contains(&needle)
                })
                .collect();
            Ok(json!({"items": items}))
        }
        "download_manager.remove" => {
            let id = params["id"]
                .as_u64()
                .ok_or_else(|| Fail::params("id is required"))?;
            let removed = state::remove_download(id).map_err(Fail::params)?;
            Ok(
                json!({"id": id, "removed": removed, "note": "the entry leaves the list; the file stays in the download folder"}),
            )
        }
        "browser_shortcut.configure" => {
            operator(caller, "changing keyboard shortcuts")?;
            const ACTIONS: [&str; 16] = [
                "focus_address",
                "new_tab",
                "close_tab",
                "reopen_tab",
                "next_tab",
                "prev_tab",
                "reload",
                "hard_reload",
                "back",
                "forward",
                "history",
                "downloads",
                "bookmark",
                "find",
                "print",
                "agent",
            ];
            let mut map = state::settings_snapshot()["shortcuts"]
                .as_object()
                .cloned()
                .unwrap_or_default();
            if let Some(keys) = params["remove"].as_str() {
                map.remove(&normalize_keys(keys)?);
            } else {
                let action = params["action"]
                    .as_str()
                    .filter(|a| ACTIONS.contains(a))
                    .ok_or_else(|| {
                        Fail::params(format!("action: one of {}", ACTIONS.join(", ")))
                    })?;
                let keys = normalize_keys(
                    params["keys"]
                        .as_str()
                        .ok_or_else(|| Fail::params("keys such as Ctrl+K or Alt+Shift+N"))?,
                )?;
                map.insert(keys, json!(action));
            }
            call(
                caller,
                "settings.set",
                json!({"settings": {"shortcuts": map}}),
            )
            .await?;
            Ok(json!({"shortcuts": map}))
        }
        "script.user_list" => Ok(
            json!({"scripts": user_scripts().into_iter().map(|s| json!({"name": s["name"], "matches": s["matches"], "enabled": s["enabled"] != false, "bytes": s["code"].as_str().map_or(0, str::len)})).collect::<Vec<_>>()}),
        ),
        "script.user_install" => {
            operator(caller, "installing a user script (code that runs on pages)")?;
            let name: String = params["name"]
                .as_str()
                .ok_or_else(|| Fail::params("name is required"))?
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                .take(48)
                .collect();
            let code = params["code"]
                .as_str()
                .ok_or_else(|| Fail::params("code is required"))?;
            let matches: Vec<String> = params["matches"]
                .as_array()
                .map(|m| {
                    m.iter()
                        .filter_map(|p| p.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            if matches.is_empty() || name.is_empty() {
                return Err(Fail::params("name and matches (Chrome match patterns such as https://*.example.com/*) are required"));
            }
            let mut list = user_scripts();
            list.retain(|s| s["name"] != name.as_str());
            list.push(json!({"name": name, "matches": matches, "code": code, "enabled": true}));
            save_user_scripts(&list)?;
            Ok(json!({"installed": name, "matches": matches}))
        }
        "script.user_remove" | "script.user_enable" | "script.user_disable" => {
            // Turning a script off is always allowed; turning it on is the person's.
            if method != "script.user_disable" {
                operator(caller, "turning on or removing a user script")?;
            }
            let name = params["name"]
                .as_str()
                .ok_or_else(|| Fail::params("name is required"))?;
            let mut list = user_scripts();
            let before = list.len();
            match method {
                "script.user_remove" => list.retain(|s| s["name"] != name),
                _ => {
                    for script in list.iter_mut().filter(|s| s["name"] == name) {
                        script["enabled"] = json!(method == "script.user_enable");
                    }
                }
            }
            if !list.iter().any(|s| s["name"] == name) && before == list.len() {
                return Err(Fail::params(format!("no user script named {name}")));
            }
            save_user_scripts(&list)?;
            Ok(json!({"name": name, "done": method.trim_start_matches("script.user_")}))
        }
        "collect.paginate" | "collect.resume" => {
            let name = params["name"].as_str().unwrap_or("pages").to_string();
            let mut job = if method == "collect.resume" {
                PAGES
                    .with(|p| {
                        p.borrow()
                            .get(&format!("{}/{name}", caller.session))
                            .cloned()
                    })
                    .ok_or_else(|| Fail::params(format!("nothing to resume under {name}")))?
            } else {
                let next = params.get("next").cloned().ok_or_else(|| Fail::params("next: how to find the next-page control, such as {by: 'text', text: 'Next'}"))?;
                json!({"next": next, "items": params.get("items").cloned().unwrap_or(json!({"by": "css", "selector": "a[href]"})), "pages": params["pages"].as_u64().unwrap_or(5).clamp(1, 50), "done": 0, "results": []})
            };
            let tab = page::tab_of(caller, params)?;
            let pages = job["pages"].as_u64().unwrap_or(5);
            while job["done"].as_u64().unwrap_or(0) < pages {
                let mut items_query = job["items"].clone();
                items_query["tabId"] = json!(tab);
                let by = items_query["by"].as_str().unwrap_or("css").to_string();
                let found = call(
                    caller,
                    &format!("locate.{}", if by == "css" { "all" } else { &by }),
                    items_query,
                )
                .await?;
                let lines: Vec<Value> = found["matches"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|m| m["line"].clone())
                    .collect();
                let page_no = job["done"].as_u64().unwrap_or(0) + 1;
                job["results"]
                    .as_array_mut()
                    .expect("results")
                    .push(json!({"page": page_no, "url": state::tab_url(&tab), "items": lines}));
                job["done"] = json!(page_no);
                PAGES.with(|p| {
                    p.borrow_mut()
                        .insert(format!("{}/{name}", caller.session), job.clone())
                });
                if page_no >= pages {
                    break;
                }
                // The next-page control is found again on each page.
                let mut next = json!({"tabId": tab, "find": job["next"]});
                let click = match tasks::resolve(caller, &mut next).await {
                    Ok(()) => call(
                        caller,
                        "act.click",
                        json!({"tabId": tab, "ref": next["ref"]}),
                    )
                    .await
                    .map(|_| ()),
                    Err(fail) => Err(fail),
                };
                if click.is_err() {
                    job["stopped"] = json!("no next page");
                    break;
                }
                let _ = call(
                    caller,
                    "wait.settle",
                    json!({"tabId": tab, "timeout": 8_000}),
                )
                .await;
            }
            PAGES.with(|p| {
                p.borrow_mut()
                    .insert(format!("{}/{name}", caller.session), job.clone())
            });
            Ok(job)
        }
        "research.compare_sources" => {
            let urls: Vec<String> = params["urls"]
                .as_array()
                .map(|u| {
                    u.iter()
                        .filter_map(|x| x.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            if urls.len() < 2 || urls.len() > 6 {
                return Err(Fail::params("urls: 2 to 6 addresses to compare"));
            }
            let lanes: Vec<Value> = urls.iter().map(|u| json!({"url": u, "steps": [{"method": "wait.load", "params": {"timeout": 20_000}}, {"method": "observe.text", "params": {"max": 40_000}}]})).collect();
            let ran = call(caller, "agent.parallel_tabs", json!({"lanes": lanes})).await?;
            let terms: Vec<String> = params["terms"]
                .as_array()
                .map(|t| {
                    t.iter()
                        .filter_map(|x| x.as_str().map(|s| s.to_lowercase()))
                        .collect()
                })
                .unwrap_or_default();
            let mut sources = Vec::new();
            let mut vocab: Vec<std::collections::HashSet<String>> = Vec::new();
            for (index, lane) in ran["lanes"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .iter()
                .enumerate()
            {
                let text = lane["results"]
                    .as_array()
                    .and_then(|r| r.last())
                    .and_then(|r| r["result"]["text"].as_str())
                    .unwrap_or("")
                    .to_string();
                let lower = text.to_lowercase();
                let words: std::collections::HashSet<String> = lower
                    .split(|c: char| !c.is_alphanumeric())
                    .filter(|w| w.len() > 4)
                    .map(str::to_string)
                    .collect();
                let numbers: Vec<String> = text
                    .split_whitespace()
                    .filter(|w| w.chars().any(|c| c.is_ascii_digit()))
                    .map(|w| {
                        w.trim_matches(|c: char| !c.is_alphanumeric() && c != '.' && c != '%')
                            .to_string()
                    })
                    .filter(|w| !w.is_empty())
                    .take(30)
                    .collect();
                sources.push(json!({
                    "url": urls[index],
                    "ok": lane["status"] == "done",
                    "chars": text.chars().count(),
                    "terms": terms.iter().map(|t| json!({"term": t, "mentions": lower.matches(t.as_str()).count()})).collect::<Vec<_>>(),
                    "numbers": numbers,
                    "excerpt": text.chars().take(300).collect::<String>(),
                }));
                vocab.push(words);
            }
            let shared: Vec<String> = match vocab.split_first() {
                Some((first, rest)) => {
                    let mut common: Vec<String> = first
                        .iter()
                        .filter(|w| rest.iter().all(|v| v.contains(*w)))
                        .cloned()
                        .collect();
                    common.sort();
                    common.into_iter().take(40).collect()
                }
                None => Vec::new(),
            };
            Ok(
                json!({"sources": sources, "sharedWords": shared, "agreement": if vocab.is_empty() { 0.0 } else { shared.len() as f64 / vocab.iter().map(|v| v.len().max(1)).min().unwrap_or(1) as f64 }}),
            )
        }
        "policy.profile_scope" => Ok(json!({
            "current": state::profile_info(),
            "profiles": call(caller, "profile.storage_partition", json!({})).await?["profiles"],
            "rule": "agents use every profile the person has not locked (profile.lock); saved sign-ins only in main or bound profiles",
        })),
        "policy.file_scope" => Ok(json!({
            "uploadFolders": state::settings_snapshot()["uploadDirs"],
            "uploadLaunchFolders": state::state().options.upload_dirs.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
            "downloadFolder": state::download_root().display().to_string(),
            "downloadsEnabled": state::downloads_enabled(),
            "rule": "agents read and upload files only in these folders and save only to the download folder",
        })),
        "policy.network_scope" => Ok(json!({
            "allowlist": call(caller, "policy.domain_allowlist", json!({})).await?["domains"],
            "blocklist": call(caller, "policy.domain_blocklist", json!({})).await?["domains"],
            "sideProfileProxies": call(caller, "profile.storage_partition", json!({})).await?["profiles"].as_array().map(|all| all.iter().filter(|p| !p["preferences"]["proxy"].is_null()).map(|p| json!({"profile": p["profile"], "proxy": p["preferences"]["proxy"]})).collect::<Vec<_>>()),
        })),
        "policy.permission_scope" => Ok(json!({
            "sites": super::extras::list()["sites"],
            "default": "refuse",
            "controlMode": state::control_mode(),
            "nativeControl": state::settings_snapshot()["nativeControl"] == true,
            "allowEvaluate": state::option_enabled("allowEvaluate"),
        })),
        "handoff.cancel" => {
            let tab = page::tab_of(caller, params)?;
            if sessions::handoff(&tab).is_none() {
                return Err(Fail::params("there is no human check on this tab"));
            }
            sessions::withdraw_handoff(&tab);
            state::notify_shell_agent();
            Ok(json!({"tabId": tab, "cancelled": true}))
        }
        "api.act" => {
            let step = params["method"]
                .as_str()
                .ok_or_else(|| Fail::params("method and params of the one action to run"))?;
            let preview = call(caller, "agent.preview", json!({"steps": [{"method": step, "params": params.get("params").cloned().unwrap_or(json!({}))}]})).await?;
            let started = std::time::Instant::now();
            let mut step_params = params.get("params").cloned().unwrap_or(json!({}));
            tasks::resolve(caller, &mut step_params).await?;
            let result = call(caller, step, step_params).await;
            let ms = started.elapsed().as_millis() as u64;
            match result {
                Ok(value) => Ok(
                    json!({"ok": true, "method": step, "risk": preview["plan"][0]["risk"], "ms": ms, "result": value}),
                ),
                Err(fail) => Ok(
                    json!({"ok": false, "method": step, "ms": ms, "error": {"code": fail.code.as_str(), "message": fail.message, "data": fail.data}, "recovery": tasks::classify(fail.code.as_str(), &fail.message)}),
                ),
            }
        }
        "benchmark.browsergym" => {
            let tab = page::tab_of(caller, params)?;
            let mut error = Value::Null;
            if let Some(action) = params["action"].as_str() {
                let (name, args) = parse_gym(action)?;
                let arg = |i: usize| args.get(i).cloned().unwrap_or_default();
                let bid = |i: usize| {
                    arg(i).trim_matches(['[', ']']).parse::<u64>().map_err(|_| {
                        Fail::params(format!("{name} needs an element id such as '12'"))
                    })
                };
                let routed: Result<(&str, Value), Fail> = match name.as_str() {
                    "click" => bid(0).map(|r| ("act.click", json!({"ref": r}))),
                    "dblclick" => bid(0).map(|r| ("act.double_click", json!({"ref": r}))),
                    "hover" => bid(0).map(|r| ("act.hover", json!({"ref": r}))),
                    "fill" => bid(0).map(|r| ("act.fill", json!({"ref": r, "text": arg(1)}))),
                    "select_option" => {
                        bid(0).map(|r| ("act.select_option", json!({"ref": r, "value": arg(1)})))
                    }
                    "press" | "keyboard_press" if args.len() >= 2 => {
                        bid(0).map(|r| ("act.press", json!({"ref": r, "key": arg(1)})))
                    }
                    "press" | "keyboard_press" => Ok(("keyboard.press", json!({"key": arg(0)}))),
                    "keyboard_type" => Ok(("keyboard.type", json!({"text": arg(0)}))),
                    "scroll" => Ok((
                        "act.scroll",
                        json!({"deltaX": arg(0).parse::<f64>().unwrap_or(0.0), "deltaY": arg(1).parse::<f64>().unwrap_or(0.0)}),
                    )),
                    "goto" => Ok(("navigate.url", json!({"url": arg(0)}))),
                    "go_back" => Ok(("tab.back", json!({}))),
                    "go_forward" => Ok(("tab.forward", json!({}))),
                    "new_tab" => Ok(("tabs.create", json!({"url": "about:blank"}))),
                    "noop" => Ok((
                        "wait.settle",
                        json!({"timeout": arg(0).parse::<u64>().unwrap_or(1000)}),
                    )),
                    "send_msg_to_user" => Ok(("window.agent_overlay", json!({"text": arg(0)}))),
                    other => Err(Fail::params(format!(
                        "{other} is not a BrowserGym action AgentGo maps"
                    ))),
                };
                let (target, mut action_params) = routed?;
                if let Some(map) = action_params.as_object_mut() {
                    map.insert("tabId".into(), json!(tab));
                }
                if let Err(fail) = call(caller, target, action_params).await {
                    error = json!(format!("{}: {}", fail.code.as_str(), fail.message));
                }
            }
            // BrowserGym's observation: the page as an accessibility tree with
            // element ids, its address, its title and the last action's error.
            let view = page::observe_page(
                caller,
                &json!({"tabId": tab, "max": params["max"].as_u64().unwrap_or(12_000)}),
            )
            .await?;
            let mut observation = json!({
                "url": state::tab_url(&tab),
                "axtree_txt": view["page"],
                "last_action_error": error,
                "open_pages_urls": state::content_tab_ids().iter().filter(|t| sessions::can_see_tab(caller, t)).map(|t| state::tab_url(t)).collect::<Vec<_>>(),
            });
            if params["screenshot"] == true {
                observation["screenshot"] =
                    page::screenshot(caller, "observe.screenshot", &json!({"tabId": tab})).await?
                        ["path"]
                        .clone();
            }
            Ok(observation)
        }
        _ => Err(Fail::internal(format!("{method} is not handled here"))),
    }
}

/// The type of an image by its first bytes: (MIME type, extension).
fn image_kind(bytes: &[u8]) -> Option<(&'static str, &'static str)> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        Some(("image/png", "png"))
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(("image/jpeg", "jpg"))
    } else if bytes.starts_with(b"GIF8") {
        Some(("image/gif", "gif"))
    } else if bytes.len() > 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some(("image/webp", "webp"))
    } else {
        None
    }
}

/// "ctrl+shift+k" -> "Ctrl+Shift+K".
pub fn normalize_keys(keys: &str) -> Result<String, Fail> {
    let mut mods = Vec::new();
    let mut key = None;
    for part in keys.split('+').map(str::trim).filter(|p| !p.is_empty()) {
        match part.to_lowercase().as_str() {
            "ctrl" | "control" => mods.push("Ctrl"),
            "shift" => mods.push("Shift"),
            "alt" => mods.push("Alt"),
            other if other.len() == 1 && other.chars().all(|c| c.is_ascii_alphanumeric()) => key = Some(other.to_uppercase()),
            other if other.starts_with('f') && other[1..].parse::<u8>().is_ok_and(|n| (1..=12).contains(&n)) => key = Some(other.to_uppercase()),
            _ => return Err(Fail::params(format!("{part} is not a key AgentGo can bind (letters, digits, F1-F12 with Ctrl, Shift, Alt)"))),
        }
    }
    let key = key.ok_or_else(|| Fail::params("name one key, such as Ctrl+K"))?;
    if mods.is_empty() && !key.starts_with('F') {
        return Err(Fail::params("a letter or digit needs Ctrl or Alt"));
    }
    let ordered: Vec<&str> = ["Ctrl", "Alt", "Shift"]
        .into_iter()
        .filter(|m| mods.contains(m))
        .collect();
    Ok(ordered
        .into_iter()
        .chain(std::iter::once(key.as_str()))
        .collect::<Vec<_>>()
        .join("+"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gym_actions_parse() {
        assert_eq!(
            parse_gym("click('12')").unwrap(),
            ("click".into(), vec!["12".into()])
        );
        assert_eq!(
            parse_gym("fill('3', 'a, b')").unwrap().1,
            vec!["3".to_string(), "a, b".to_string()]
        );
        assert_eq!(parse_gym("noop()").unwrap().1.len(), 0);
        assert_eq!(
            parse_gym(r#"send_msg_to_user("it's \"done\"")"#).unwrap().1[0],
            "it's \"done\""
        );
    }

    #[test]
    fn shortcut_keys_normalize() {
        assert_eq!(normalize_keys("shift+ctrl+k").unwrap(), "Ctrl+Shift+K");
        assert_eq!(normalize_keys("F7").unwrap(), "F7");
        assert!(normalize_keys("k").is_err());
        assert!(normalize_keys("Ctrl+Space").is_err());
    }

    #[test]
    fn redaction_catches_secrets() {
        let (text, count) = redact("mail ada@example.com token=abc123 card 4111111111111111 Bearer eyJhbGciOiJIUzI1NiJ9.e30.x ok");
        assert!(
            !text.contains("ada@example.com")
                && !text.contains("abc123")
                && !text.contains("4111")
                && !text.contains("eyJ")
        );
        assert!(count >= 4 && text.ends_with("ok"));
    }
}
