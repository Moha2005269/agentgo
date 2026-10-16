//! Native dialogs a page opens while an agent drives it, held for the agent
//! instead of popping up on the person's screen: the file picker
//! (`file.dialog_*`, `file.choose_*`, `dialog.file`) and HTTP sign-in
//! (`dialog.auth`). Also the waits for a download, a popup or a JavaScript
//! dialog.
//!
//! Policy: an agent may only choose files in the upload folders the person
//! granted, and save into the download folder. HTTP sign-in is answered with
//! the person's saved sign-in for that site (the agent never sees it) or by
//! the person; an agent can only cancel it.

use std::cell::RefCell;
use std::collections::BTreeMap;

use agentgo_protocol::ErrorCode;
use cef::{ImplAuthCallback, ImplFileDialogCallback};
use serde_json::{json, Value};

use super::sessions::{self, Caller, Fail, Reply};
use super::{cdp, exec, page, state};

struct FileDialog {
    mode: i32,
    title: String,
    default_path: String,
    filters: Vec<String>,
    callback: cef::FileDialogCallback,
}

/// Who answers a held sign-in: CEF's callback or a DevTools request.
enum Answer {
    Cef(cef::AuthCallback),
    Fetch(String),
}

struct Auth {
    origin: String,
    proxy: bool,
    host: String,
    port: i32,
    realm: String,
    scheme: String,
    callback: Answer,
}

impl Answer {
    fn give(self, tab: &str, credentials: Option<(&str, &str)>) {
        match self {
            Answer::Cef(callback) => match credentials {
                Some((user, password)) => callback.cont(
                    Some(&cef::CefString::from(user)),
                    Some(&cef::CefString::from(password)),
                ),
                None => callback.cancel(),
            },
            Answer::Fetch(request) => {
                let response = match credentials {
                    Some((user, password)) => {
                        json!({"response": "ProvideCredentials", "username": user, "password": password})
                    }
                    None => json!({"response": "CancelAuth"}),
                };
                let tab = tab.to_string();
                exec::spawn(async move {
                    let _ = cdp::call(
                        &tab,
                        "Fetch.continueWithAuth",
                        json!({"requestId": request, "authChallengeResponse": response}),
                    )
                    .await;
                });
            }
        }
    }
}

thread_local! {
    static FILES: RefCell<BTreeMap<String, FileDialog>> = const { RefCell::new(BTreeMap::new()) };
    static AUTH: RefCell<BTreeMap<String, Auth>> = const { RefCell::new(BTreeMap::new()) };
    /// tab -> origins whose sign-in prompts DevTools hands to AgentGo.
    static ARMED: RefCell<BTreeMap<String, Vec<String>>> = const { RefCell::new(BTreeMap::new()) };
}

const MODES: [&str; 4] = ["open", "open_multiple", "folder", "save"];

pub fn hold_file_dialog(
    tab: &str,
    mode: i32,
    title: String,
    default_path: String,
    filters: Vec<String>,
    callback: cef::FileDialogCallback,
) {
    // A second picker replaces an unanswered first one.
    if let Some(old) = FILES.with(|f| f.borrow_mut().remove(tab)) {
        old.callback.cancel();
    }
    FILES.with(|f| {
        f.borrow_mut().insert(
            tab.to_string(),
            FileDialog {
                mode,
                title,
                default_path,
                filters,
                callback,
            },
        )
    });
    sessions::emit(
        "dialog.file",
        json!({"tabId": tab, "mode": MODES.get(mode as usize).copied().unwrap_or("open")}),
    );
}

#[allow(clippy::too_many_arguments)]
pub fn auth_challenge(
    tab: &str,
    origin: String,
    proxy: bool,
    host: String,
    port: i32,
    realm: String,
    scheme: String,
    callback: cef::AuthCallback,
) {
    hold_auth(
        tab,
        origin,
        proxy,
        host,
        port,
        realm,
        scheme,
        Answer::Cef(callback),
    );
}

/// A site asked for HTTP sign-in and Chromium gave up: from now on DevTools
/// hands this origin's prompts to AgentGo, and the page loads again.
/// False when the origin was armed already (then the error stands).
pub fn arm_auth(tab: &str, url: &str) -> bool {
    let Some(origin) = super::dispatch::web_origin(url) else {
        return false;
    };
    let fresh = ARMED.with(|a| {
        let mut all = a.borrow_mut();
        let list = all.entry(tab.to_string()).or_default();
        if list.contains(&origin) {
            false
        } else {
            list.push(origin.clone());
            true
        }
    });
    if !fresh {
        return false;
    }
    let (tab, url) = (tab.to_string(), url.to_string());
    exec::spawn(async move {
        let patterns: Vec<Value> = ARMED
            .with(|a| a.borrow().get(&tab).cloned().unwrap_or_default())
            .iter()
            .map(|o| json!({"urlPattern": format!("{o}/*")}))
            .collect();
        if cdp::call(
            &tab,
            "Fetch.enable",
            json!({"patterns": patterns, "handleAuthRequests": true}),
        )
        .await
        .is_ok()
        {
            state::navigate_tab(&tab, &url);
        }
    });
    true
}

/// DevTools `Fetch.authRequired` for an armed origin.
pub async fn fetch_auth(tab: String, params: Value) {
    let challenge = &params["authChallenge"];
    let origin = challenge["origin"].as_str().unwrap_or("").to_string();
    let host = origin
        .split("://")
        .nth(1)
        .unwrap_or(&origin)
        .split(':')
        .next()
        .unwrap_or("")
        .to_string();
    let port = origin
        .rsplit(':')
        .next()
        .and_then(|p| p.parse().ok())
        .unwrap_or(0);
    hold_auth(
        &tab,
        origin,
        challenge["source"] == "Proxy",
        host,
        port,
        challenge["realm"].as_str().unwrap_or("").to_string(),
        challenge["scheme"].as_str().unwrap_or("").to_string(),
        Answer::Fetch(params["requestId"].as_str().unwrap_or("").to_string()),
    );
}

#[allow(clippy::too_many_arguments)]
fn hold_auth(
    tab: &str,
    origin: String,
    proxy: bool,
    host: String,
    port: i32,
    realm: String,
    scheme: String,
    callback: Answer,
) {
    // The person's saved sign-in for this site answers at once.
    if !proxy {
        if let Some((user, password)) = super::accounts::saved_login(&origin) {
            callback.give(tab, Some((&user, &password)));
            state::log_event(&format!("AG-RUNTIME auth-filled tab={tab} host={host:?}"));
            return;
        }
    }
    if let Some(old) = AUTH.with(|a| a.borrow_mut().remove(tab)) {
        old.callback.give(tab, None);
    }
    AUTH.with(|a| {
        a.borrow_mut().insert(
            tab.to_string(),
            Auth {
                origin,
                proxy,
                host: host.clone(),
                port,
                realm: realm.clone(),
                scheme,
                callback,
            },
        )
    });
    sessions::emit(
        "dialog.auth",
        json!({"tabId": tab, "host": host, "realm": realm}),
    );
    state::notify_operator(&format!("{host} asks for a user name and password"));
}

fn describe_file(tab: &str, dialog: &FileDialog) -> Value {
    json!({
        "tabId": tab,
        "mode": MODES.get(dialog.mode as usize).copied().unwrap_or("open"),
        "title": dialog.title,
        "defaultPath": dialog.default_path,
        "accept": dialog.filters,
    })
}

fn paths_of(params: &Value) -> Vec<String> {
    match params.get("paths").or_else(|| params.get("path")) {
        Some(Value::String(path)) => vec![path.clone()],
        Some(Value::Array(list)) => list
            .iter()
            .filter_map(|p| p.as_str().map(str::to_string))
            .collect(),
        _ => Vec::new(),
    }
}

/// Answers the tab's held file picker with `paths` (checked against policy).
fn answer_files(caller: &Caller, tab: &str, paths: Vec<String>, want: Option<&str>) -> Reply {
    let dialog = FILES.with(|f| f.borrow_mut().remove(tab)).ok_or_else(|| {
        Fail::params(
            "no file dialog is open on this tab; wait.dialog or dialog.file shows when one is",
        )
    })?;
    let mode = MODES.get(dialog.mode as usize).copied().unwrap_or("open");
    let put_back =
        |dialog: FileDialog| FILES.with(|f| f.borrow_mut().insert(tab.to_string(), dialog));
    if let Some(want) = want {
        if want != mode && !(want == "open" && mode == "open_multiple") {
            let fail = Fail::params(format!("the open dialog is a {mode} dialog"));
            put_back(dialog);
            return Err(fail);
        }
    }
    let problem = if paths.is_empty() {
        Some("name the file or files: paths".to_string())
    } else if mode != "open_multiple" && paths.len() > 1 {
        Some(format!("a {mode} dialog takes one path"))
    } else {
        paths.iter().find_map(|path| {
            let p = std::path::Path::new(path);
            match mode {
                "save" => {
                    let root = state::download_root();
                    let inside = p
                        .parent()
                        .and_then(|d| d.canonicalize().ok())
                        .zip(root.canonicalize().ok())
                        .is_some_and(|(d, r)| d.starts_with(r));
                    (!caller.is_shell() && !inside).then(|| {
                        format!("{path} is outside the download folder {}", root.display())
                    })
                }
                "folder" => (!p.is_dir())
                    .then(|| format!("{path} is not a folder"))
                    .or_else(|| {
                        (!caller.is_shell() && !state::upload_allowed(path)).then(|| {
                            format!("{path} is outside the upload folders the person granted")
                        })
                    }),
                _ => (!p.is_file())
                    .then(|| format!("{path} is not a file"))
                    .or_else(|| {
                        (!caller.is_shell() && !state::upload_allowed(path)).then(|| {
                            format!("{path} is outside the upload folders the person granted")
                        })
                    }),
            }
        })
    };
    if let Some(problem) = problem {
        let code = if problem.contains("outside") {
            ErrorCode::PolicyDenied
        } else {
            ErrorCode::InvalidParams
        };
        put_back(dialog);
        return Err(Fail::new(code, problem));
    }
    let mut list = cef::CefStringList::new();
    for path in &paths {
        list.append(path);
    }
    dialog.callback.cont(Some(&mut list));
    Ok(json!({"tabId": tab, "mode": mode, "chosen": paths}))
}

pub fn handles(method: &str) -> bool {
    matches!(
        method,
        "file.dialog_open"
            | "file.dialog_save"
            | "file.dialog_cancel"
            | "file.dialog_confirm"
            | "file.choose_multiple"
            | "file.choose_directory"
            | "dialog.file"
            | "dialog.auth"
            | "wait.download"
            | "wait.popup"
            | "wait.dialog"
    )
}

pub async fn command(caller: &Caller, method: &str, params: &Value) -> Reply {
    match method {
        "dialog.file" | "file.dialog_open" if paths_of(params).is_empty() => {
            let tab = page::tab_of(caller, params)?;
            Ok(FILES
                .with(|f| {
                    f.borrow()
                        .get(&tab)
                        .map(|d| json!({"open": true, "dialog": describe_file(&tab, d)}))
                })
                .unwrap_or_else(|| json!({"open": false})))
        }
        "dialog.file" | "file.dialog_open" | "file.dialog_confirm" | "file.choose_multiple" => {
            let tab = page::tab_of(caller, params)?;
            let want = match method {
                "file.choose_multiple" => Some("open_multiple"),
                "file.dialog_open" => Some("open"),
                _ => None,
            };
            answer_files(caller, &tab, paths_of(params), want)
        }
        "file.choose_directory" => {
            let tab = page::tab_of(caller, params)?;
            answer_files(caller, &tab, paths_of(params), Some("folder"))
        }
        "file.dialog_save" => {
            let tab = page::tab_of(caller, params)?;
            let mut paths = paths_of(params);
            if paths.is_empty() {
                // The page's suggested name, in the download folder.
                let name = FILES
                    .with(|f| f.borrow().get(&tab).map(|d| d.default_path.clone()))
                    .unwrap_or_default();
                let name = std::path::Path::new(&name)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .filter(|n| !n.is_empty())
                    .unwrap_or_else(|| "download".into());
                paths.push(state::download_root().join(name).display().to_string());
            }
            answer_files(caller, &tab, paths, Some("save"))
        }
        "file.dialog_cancel" => {
            let tab = page::tab_of(caller, params)?;
            let dialog = FILES
                .with(|f| f.borrow_mut().remove(&tab))
                .ok_or_else(|| Fail::params("no file dialog is open on this tab"))?;
            dialog.callback.cancel();
            Ok(json!({"tabId": tab, "cancelled": true}))
        }
        "dialog.auth" => {
            let tab = page::tab_of(caller, params)?;
            let pending = AUTH.with(|a| a.borrow().get(&tab).map(|auth| json!({"origin": auth.origin, "host": auth.host, "port": auth.port, "realm": auth.realm, "scheme": auth.scheme, "proxy": auth.proxy})));
            if params["cancel"] == true {
                let auth = AUTH
                    .with(|a| a.borrow_mut().remove(&tab))
                    .ok_or_else(|| Fail::params("no sign-in prompt is open on this tab"))?;
                auth.callback.give(&tab, None);
                return Ok(json!({"tabId": tab, "cancelled": true}));
            }
            if params.get("username").is_some() {
                // Typing a password is the person's; agents use saved sign-ins.
                if !caller.is_shell() {
                    return Err(Fail::new(ErrorCode::PolicyDenied, "agents never type passwords; the person saves the sign-in in the Account Center, or answers the prompt"));
                }
                let auth = AUTH
                    .with(|a| a.borrow_mut().remove(&tab))
                    .ok_or_else(|| Fail::params("no sign-in prompt is open on this tab"))?;
                auth.callback.give(
                    &tab,
                    Some((
                        params["username"].as_str().unwrap_or(""),
                        params["password"].as_str().unwrap_or(""),
                    )),
                );
                return Ok(json!({"tabId": tab, "answered": true}));
            }
            Ok(
                json!({"tabId": tab, "open": pending.is_some(), "prompt": pending, "next": "the person answers it (or saves the sign-in in the Account Center); cancel: true gives up"}),
            )
        }
        "wait.dialog" => {
            let tab = page::tab_of(caller, params)?;
            let timeout = params["timeout"]
                .as_u64()
                .unwrap_or(8_000)
                .clamp(100, 60_000);
            let started = std::time::Instant::now();
            loop {
                if let Some(dialog) = cdp::dialog(&tab) {
                    return Ok(
                        json!({"kind": "javascript", "type": dialog["type"], "message": dialog["message"], "next": "dialog.accept or dialog.dismiss"}),
                    );
                }
                if let Some(file) =
                    FILES.with(|f| f.borrow().get(&tab).map(|d| describe_file(&tab, d)))
                {
                    return Ok(
                        json!({"kind": "file", "dialog": file, "next": "file.dialog_confirm {paths} or file.dialog_cancel"}),
                    );
                }
                if AUTH.with(|a| a.borrow().contains_key(&tab)) {
                    return Ok(json!({"kind": "auth", "next": "dialog.auth"}));
                }
                if started.elapsed().as_millis() as u64 >= timeout {
                    return Err(Fail::new(
                        ErrorCode::Timeout,
                        format!("no dialog opened within {timeout} ms"),
                    ));
                }
                exec::sleep(60).await;
            }
        }
        "wait.popup" => {
            let timeout = params["timeout"]
                .as_u64()
                .unwrap_or(8_000)
                .clamp(100, 60_000);
            let before = state::content_tab_ids();
            let started = std::time::Instant::now();
            loop {
                if let Some(tab) = state::content_tab_ids()
                    .into_iter()
                    .find(|t| !before.contains(t) && sessions::can_see_tab(caller, t))
                {
                    sessions::authorize_tab(caller, &tab)?;
                    return Ok(json!({"tabId": tab, "url": state::tab_url(&tab)}));
                }
                if started.elapsed().as_millis() as u64 >= timeout {
                    return Err(Fail::new(ErrorCode::Timeout, format!("no popup opened within {timeout} ms; recovery.popup_blocked shows blocked ones")));
                }
                exec::sleep(60).await;
            }
        }
        "wait.download" => {
            let timeout = params["timeout"]
                .as_u64()
                .unwrap_or(30_000)
                .clamp(100, 600_000);
            let started = std::time::Instant::now();
            // Downloads that were there before (given, or finished by now) are not "the next one".
            let known: Vec<Value> = match params["known"].as_array() {
                Some(ids) => ids.clone(),
                None => state::downloads_snapshot()
                    .iter()
                    .filter(|d| d["inProgress"] != true)
                    .map(|d| d["id"].clone())
                    .collect(),
            };
            loop {
                let list = state::downloads_snapshot();
                let wanted = |d: &&Value| match params["id"].as_u64() {
                    Some(id) => d["id"] == id,
                    None => !known.contains(&d["id"]),
                };
                if let Some(done) = list.iter().filter(wanted).find(|d| d["inProgress"] != true) {
                    return Ok(
                        json!({"download": done, "complete": done["status"] == "complete" || done["isComplete"] == true}),
                    );
                }
                if started.elapsed().as_millis() as u64 >= timeout {
                    return Err(Fail::new(
                        ErrorCode::Timeout,
                        format!("no download finished within {timeout} ms"),
                    )
                    .with(json!({"downloads": list})));
                }
                exec::sleep(100).await;
            }
        }
        _ => Err(Fail::internal(format!("{method} is not a dialog command"))),
    }
}
