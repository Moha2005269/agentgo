//! Things that were first declared impossible, made possible with the person
//! kept in charge:
//!
//! - site permissions the person grants per site (camera, microphone,
//!   location, notifications, ...); everything else stays refused;
//! - the DevTools window and the system print dialog, for the person to use;
//! - opening a download, revealing it in File Explorer, opening a non-web
//!   link (mailto:, tel:, ...) or another program: each needs the person's
//!   approval for that exact action;
//! - a virtual passkey authenticator (WebAuthn) for sign-in flows;
//! - recording what a tab shows, as a flip-book of frames.

use std::collections::BTreeMap;

use agentgo_protocol::ErrorCode;
use serde_json::{json, Value};

use super::sessions::{self, Caller, Fail, Reply};
use super::{cdp, exec, page, state};

/// Permission names and their Chromium request bits.
const KINDS: [(&str, u32); 14] = [
    ("camera", 4 | 2),
    ("microphone", 4096),
    ("geolocation", 256),
    ("notifications", 32768),
    ("clipboard", 16),
    ("midi", 8192),
    ("storage", 1_048_576 | 32 | 64),
    ("fonts", 128),
    ("window-management", 8_388_608),
    ("sensors", 268_435_456),
    ("idle-detection", 2048),
    ("pointer-lock", 131_072),
    ("keyboard-lock", 65_536),
    ("downloads", 16_384),
];

fn site_permissions() -> BTreeMap<String, Vec<String>> {
    serde_json::from_value(state::settings_snapshot()["sitePermissions"].clone())
        .unwrap_or_default()
}

fn origin_of(text: &str) -> Result<String, Fail> {
    let (scheme, rest) = text
        .split_once("://")
        .ok_or_else(|| Fail::params("params.origin must be like https://example.com"))?;
    if !matches!(scheme, "https" | "http") {
        return Err(Fail::params(
            "permissions are granted to http and https sites",
        ));
    }
    let host = rest
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    if host.is_empty()
        || !host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-:[]".contains(&b))
    {
        return Err(Fail::params("params.origin has no valid host"));
    }
    Ok(format!("{scheme}://{host}"))
}

fn kinds_of(params: &Value) -> Result<Vec<String>, Fail> {
    let Some(list) = params.get("kinds").and_then(Value::as_array) else {
        return Ok(KINDS.iter().map(|(name, _)| name.to_string()).collect());
    };
    list.iter()
        .map(|kind| {
            let name = kind.as_str().unwrap_or("");
            KINDS
                .iter()
                .find(|(known, _)| *known == name)
                .map(|(known, _)| known.to_string())
                .ok_or_else(|| {
                    Fail::params(format!(
                        "unknown permission {name}; use {}",
                        KINDS.map(|(n, _)| n).join(", ")
                    ))
                })
        })
        .collect()
}

/// Chromium's stored site settings for a permission kind.
fn content_types(kind: &str) -> Vec<cef::sys::cef_content_setting_types_t> {
    use cef::sys::cef_content_setting_types_t as T;
    match kind {
        "camera" => vec![
            T::CEF_CONTENT_SETTING_TYPE_MEDIASTREAM_CAMERA,
            T::CEF_CONTENT_SETTING_TYPE_CAMERA_PAN_TILT_ZOOM,
        ],
        "microphone" => vec![T::CEF_CONTENT_SETTING_TYPE_MEDIASTREAM_MIC],
        "geolocation" => vec![T::CEF_CONTENT_SETTING_TYPE_GEOLOCATION],
        "notifications" => vec![T::CEF_CONTENT_SETTING_TYPE_NOTIFICATIONS],
        "clipboard" => vec![T::CEF_CONTENT_SETTING_TYPE_CLIPBOARD_READ_WRITE],
        "midi" => vec![T::CEF_CONTENT_SETTING_TYPE_MIDI_SYSEX],
        "storage" => vec![T::CEF_CONTENT_SETTING_TYPE_STORAGE_ACCESS],
        "fonts" => vec![T::CEF_CONTENT_SETTING_TYPE_LOCAL_FONTS],
        "window-management" => vec![T::CEF_CONTENT_SETTING_TYPE_WINDOW_MANAGEMENT],
        "sensors" => vec![T::CEF_CONTENT_SETTING_TYPE_SENSORS],
        "idle-detection" => vec![T::CEF_CONTENT_SETTING_TYPE_IDLE_DETECTION],
        "keyboard-lock" => vec![T::CEF_CONTENT_SETTING_TYPE_KEYBOARD_LOCK],
        "pointer-lock" => vec![T::CEF_CONTENT_SETTING_TYPE_POINTER_LOCK],
        "downloads" => vec![T::CEF_CONTENT_SETTING_TYPE_AUTOMATIC_DOWNLOADS],
        _ => Vec::new(),
    }
}

/// `permission.grant` / `permission.deny` / `permission.reset` (operator).
/// The stored grants answer future requests; Chromium's own remembered answer
/// for each affected site is replaced too, so a site that was refused before
/// is asked again (and allowed) at once.
pub async fn change_permission(method: &str, params: &Value) -> Reply {
    use cef::sys::cef_content_setting_values_t as V;
    use cef::ImplRequestContext;
    let before = site_permissions();
    let out = store_permission(method, params)?;
    let after = site_permissions();
    if let Some(context) = state::content_context() {
        for origin in before.keys().chain(after.keys()) {
            let url = cef::CefString::from(format!("{origin}/").as_str());
            for (kind, _) in KINDS {
                let allowed = after
                    .get(origin)
                    .is_some_and(|kinds| kinds.iter().any(|k| k == kind));
                let value = if allowed {
                    V::CEF_CONTENT_SETTING_VALUE_ALLOW
                } else {
                    V::CEF_CONTENT_SETTING_VALUE_DEFAULT
                };
                for content_type in content_types(kind) {
                    context.set_content_setting(
                        Some(&url),
                        Some(&url),
                        content_type.into(),
                        value.into(),
                    );
                }
            }
        }
    }
    Ok(out)
}

fn store_permission(method: &str, params: &Value) -> Reply {
    let mut all = site_permissions();
    match method {
        "permission.reset" => match params.get("origin").and_then(Value::as_str) {
            Some(origin) => {
                all.remove(&origin_of(origin)?);
            }
            None => all.clear(),
        },
        _ => {
            let origin = origin_of(params["origin"].as_str().unwrap_or(""))?;
            let kinds = kinds_of(params)?;
            let granted = all.entry(origin.clone()).or_default();
            if method == "permission.grant" {
                for kind in kinds {
                    if !granted.contains(&kind) {
                        granted.push(kind);
                    }
                }
            } else {
                granted.retain(|kind| !kinds.contains(kind));
            }
            if granted.is_empty() {
                all.remove(&origin);
            }
        }
    }
    state::update_settings(&json!({"sitePermissions": all})).map_err(Fail::internal)?;
    Ok(list())
}

/// `permission.list`: what each site may use. Anything not listed is refused.
pub fn list() -> Value {
    json!({
        "default": "refuse",
        "sites": site_permissions(),
        "kinds": KINDS.map(|(name, _)| name),
        "note": "Sites may use only what the person granted (permission.grant, operator only); every other request is refused without asking.",
    })
}

/// Names of the permissions whose request bits appear in `mask`.
pub fn kinds_in_mask(mask: u32) -> Vec<&'static str> {
    KINDS
        .iter()
        .filter(|(_, bits)| mask & bits != 0)
        .map(|(name, _)| *name)
        .collect()
}

/// The request bits `origin` may use.
pub fn allowed_mask(origin: &str) -> u32 {
    let Ok(origin) = origin_of(origin.trim_end_matches('/')) else {
        return 0;
    };
    site_permissions()
        .get(&origin)
        .map(|granted| {
            KINDS
                .iter()
                .filter(|(name, _)| granted.iter().any(|kind| kind == name))
                .fold(0, |mask, (_, bits)| mask | bits)
        })
        .unwrap_or(0)
}

/// Media capture bits (audio 1, video 2) allowed for `origin`.
pub fn allowed_media(origin: &str) -> u32 {
    let mask = allowed_mask(origin);
    (if mask & 4096 != 0 { 1 } else { 0 }) | (if mask & 4 != 0 { 2 } else { 0 })
}

fn host_of(tab: &str) -> Result<cef::BrowserHost, Fail> {
    state::content_host(tab)
        .ok_or_else(|| Fail::new(ErrorCode::TargetStale, "the tab has no browser"))
}

/// `debug.devtools_open` / `debug.devtools_close`: the DevTools window of a
/// tab, for the person.
pub fn devtools(caller: &Caller, params: &Value, open: bool) -> Reply {
    use cef::ImplBrowserHost;
    let tab = page::tab_of(caller, params)?;
    let host = host_of(&tab)?;
    if open {
        host.show_dev_tools(None, None, None, None);
    } else {
        host.close_dev_tools();
    }
    Ok(json!({"tabId": tab, "open": open}))
}

/// `print.to_printer`: the system print dialog; the person chooses the printer
/// and confirms.
pub fn print_dialog(caller: &Caller, params: &Value) -> Reply {
    use cef::ImplBrowserHost;
    let tab = page::tab_of(caller, params)?;
    // The dialog holds the browser until it is answered, so it opens only
    // after the person agreed to print.
    sessions::require_approval(
        caller,
        &tab,
        "agentgo://outside",
        &format!("print {}", state::tab_url(&tab)),
        "print on paper",
        params["approvalId"].as_str(),
    )?;
    host_of(&tab)?.print();
    Ok(
        json!({"tabId": tab, "dialog": true, "note": "the print dialog is open; the person confirms it"}),
    )
}

fn download_path(params: &Value) -> Result<String, Fail> {
    let id = params["id"]
        .as_u64()
        .ok_or_else(|| Fail::params("params.id (a download id from downloads.list) is required"))?;
    let item = state::downloads_snapshot()
        .into_iter()
        .find(|item| item["id"].as_u64() == Some(id))
        .ok_or_else(|| Fail::params(format!("no download with id {id}")))?;
    if item["complete"] != true {
        return Err(Fail::new(
            ErrorCode::TargetBlocked,
            "the download has not finished",
        ));
    }
    item["path"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| Fail::internal("the download has no file"))
}

#[cfg(windows)]
pub(super) fn shell_open(target: &str, arguments: Option<&str>) -> Result<(), Fail> {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let wide = |text: &str| text.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let (verb, file) = (wide("open"), wide(target));
    let args = arguments.map(wide);
    // SAFETY: every pointer refers to a NUL-terminated buffer that lives
    // until the call returns.
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            args.as_ref().map_or(std::ptr::null(), |a| a.as_ptr()),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    if result as isize <= 32 {
        return Err(Fail::internal(format!(
            "Windows could not open {target} (code {})",
            result as isize
        )));
    }
    Ok(())
}

#[cfg(not(windows))]
pub(super) fn shell_open(_target: &str, _arguments: Option<&str>) -> Result<(), Fail> {
    Err(Fail::new(
        ErrorCode::CapabilityUnsupported,
        "only on Windows",
    ))
}

/// Actions that leave the browser: each one needs the person's approval of
/// exactly that action (`approvalId` after `approval.wait`).
pub fn outside(caller: &Caller, method: &str, params: &Value) -> Reply {
    let tab = page::tab_of(caller, params)?;
    let approval = params["approvalId"].as_str();
    let ask = |target: &str, reason: &str| {
        sessions::require_approval(caller, &tab, "agentgo://outside", target, reason, approval)
    };
    match method {
        "download.show" => {
            let path = download_path(params)?;
            std::process::Command::new("explorer.exe")
                .arg(format!("/select,{path}"))
                .spawn()
                .map_err(|error| Fail::internal(error.to_string()))?;
            Ok(json!({"shown": path}))
        }
        "download.open" => {
            let path = download_path(params)?;
            let scan = state::download_scan_status(&path);
            match scan.as_deref() {
                Some("threat") => {
                    return Err(Fail::new(
                        ErrorCode::PolicyDenied,
                        "Microsoft Defender flagged this file; it will not be opened",
                    ))
                }
                Some("scanning") => {
                    return Err(Fail::new(
                        ErrorCode::PolicyDenied,
                        "the file is still being scanned; try again in a moment",
                    ))
                }
                _ => {}
            }
            let prompt = if scan.as_deref() == Some("clean") {
                format!("open {path}")
            } else {
                format!("open {path} (it was NOT scanned)")
            };
            ask(&prompt, "open a downloaded file with its program")?;
            shell_open(&path, None)?;
            Ok(json!({"opened": path, "scanned": scan.as_deref() == Some("clean")}))
        }
        "navigate.external" => {
            let url = params["url"]
                .as_str()
                .ok_or_else(|| Fail::params("params.url is required"))?;
            let scheme = url.split(':').next().unwrap_or("").to_ascii_lowercase();
            if scheme.is_empty()
                || matches!(
                    scheme.as_str(),
                    "http" | "https" | "file" | "javascript" | "data"
                )
                || url.len() > 2048
            {
                return Err(Fail::params("navigate.external opens mailto:, tel: and other non-web links; use navigate.url for web pages"));
            }
            ask(&format!("open {url}"), "hand a link to another program")?;
            shell_open(url, None)?;
            Ok(json!({"opened": url}))
        }
        "native.open_with" => {
            let path = params["path"]
                .as_str()
                .ok_or_else(|| Fail::params("params.path is required"))?;
            let program = params["program"]
                .as_str()
                .ok_or_else(|| Fail::params("params.program is required"))?;
            ask(
                &format!("open {path} with {program}"),
                "open a file with another program",
            )?;
            shell_open(program, Some(&format!("\"{path}\"")))?;
            Ok(json!({"opened": path, "with": program}))
        }
        _ => {
            // native.external_app
            let program = params["program"]
                .as_str()
                .ok_or_else(|| Fail::params("params.program is required"))?;
            let arguments = params["arguments"].as_str();
            ask(
                &format!("run {program} {}", arguments.unwrap_or("")),
                "start another program",
            )?;
            shell_open(program, arguments)?;
            Ok(json!({"started": program}))
        }
    }
}

/// `device.web_authn`: a virtual passkey authenticator for the tab, so sign-in
/// and sign-up flows that use passkeys can be completed. `remove: true` takes
/// it away again.
pub async fn web_authn(caller: &Caller, params: &Value) -> Reply {
    let tab = page::tab_of(caller, params)?;
    let fail = |message: String| Fail::internal(message);
    cdp::call(&tab, "WebAuthn.enable", json!({"enableUI": false}))
        .await
        .map_err(fail)?;
    if let Some(id) = params["remove"].as_str() {
        cdp::call(
            &tab,
            "WebAuthn.removeVirtualAuthenticator",
            json!({"authenticatorId": id}),
        )
        .await
        .map_err(fail)?;
        return Ok(json!({"removed": id}));
    }
    let added = cdp::call(
        &tab,
        "WebAuthn.addVirtualAuthenticator",
        json!({"options": {"protocol": "ctap2", "transport": "internal", "hasResidentKey": true, "hasUserVerification": true, "isUserVerified": true, "automaticPresenceSimulation": true}}),
    )
    .await
    .map_err(fail)?;
    Ok(json!({"tabId": tab, "authenticatorId": added["authenticatorId"]}))
}

/// `observe.record_video`: what the tab shows for a few seconds, as numbered
/// frames and a page that plays them.
pub async fn record_video(caller: &Caller, params: &Value) -> Reply {
    let tab = page::tab_of(caller, params)?;
    let seconds = params["seconds"].as_f64().unwrap_or(3.0).clamp(0.5, 30.0);
    let fps = params["fps"].as_f64().unwrap_or(4.0).clamp(1.0, 10.0);
    let count = (seconds * fps).round() as usize;
    let mut frames = Vec::new();
    for _ in 0..count {
        let started = std::time::Instant::now();
        let shot = cdp::call_timeout(
            &tab,
            "Page.captureScreenshot",
            json!({"format": "jpeg", "quality": 60, "optimizeForSpeed": true}),
            10_000,
        )
        .await
        .map_err(Fail::internal)?;
        let bytes = page::decode_base64(shot["data"].as_str().unwrap_or(""));
        let saved = page::save_artifact(caller, "frame", "jpg", &bytes)?;
        frames.push(saved["path"].as_str().unwrap_or("").to_string());
        let wait = (1000.0 / fps) as u64;
        exec::sleep(wait.saturating_sub(started.elapsed().as_millis() as u64)).await;
    }
    let names: Vec<String> = frames
        .iter()
        .map(|path| format!("file:///{}", path.replace('\\', "/")))
        .collect();
    let player = format!(
        "<!doctype html><meta charset=utf-8><title>AgentGo recording</title><body style=\"margin:0;background:#111\"><img id=f style=\"width:100%\"><script>const f={};let i=0;setInterval(()=>{{document.getElementById('f').src=f[i++%f.length]}},{});</script>",
        serde_json::to_string(&names).unwrap_or_default(),
        (1000.0 / fps) as u64
    );
    let saved = page::save_artifact(caller, "recording", "html", player.as_bytes())?;
    Ok(
        json!({"tabId": tab, "frames": frames.len(), "fps": fps, "player": saved["path"], "first": frames.first()}),
    )
}

/// `sync.start {folder}` / `sync.status` / `sync.stop` (operator): folder
/// sync. Bookmarks and settings are merged with `AgentGo-sync.json` in a
/// folder the person chooses (a OneDrive or Google Drive folder syncs it
/// between computers). Passwords, cookies and tokens never go there.
pub fn sync(method: &str, params: &Value) -> Reply {
    let settings = state::settings_snapshot();
    match method {
        "sync.stop" => {
            state::update_settings(&json!({"syncFolder": null})).map_err(Fail::internal)?;
            Ok(json!({"syncing": false}))
        }
        "sync.status" => Ok(
            json!({"syncing": settings["syncFolder"].is_string(), "folder": settings["syncFolder"], "lastSync": settings["syncLast"]}),
        ),
        _ => {
            let folder = params["folder"]
                .as_str()
                .map(str::to_string)
                .or_else(|| settings["syncFolder"].as_str().map(str::to_string))
                .ok_or_else(|| Fail::params("params.folder: the folder to sync with"))?;
            let dir = std::path::PathBuf::from(&folder);
            if !dir.is_dir() {
                return Err(Fail::params(format!("{folder} is not a folder")));
            }
            let file = dir.join("AgentGo-sync.json");
            let theirs: Value = std::fs::read(&file)
                .ok()
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                .unwrap_or_else(|| json!({}));
            // Bookmarks: the union of both sides.
            let mut added = 0;
            for bookmark in theirs["bookmarks"].as_array().cloned().unwrap_or_default() {
                let (Some(url), Some(title)) =
                    (bookmark["url"].as_str(), bookmark["title"].as_str())
                else {
                    continue;
                };
                if state::is_navigable_url(url)
                    && state::with_store(|store| {
                        store.add_bookmark(url, title, bookmark["createdAt"].as_u64().unwrap_or(0))
                    })
                    .is_ok()
                {
                    added += 1;
                }
            }
            // Settings: the newer side wins; private keys stay on this computer.
            const PRIVATE: [&str; 5] = [
                "syncFolder",
                "syncLast",
                "sitePermissions",
                "profileId",
                "uploadDirs",
            ];
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            if theirs["savedAt"].as_u64().unwrap_or(0) > settings["syncLast"].as_u64().unwrap_or(0)
            {
                if let Some(map) = theirs["settings"].as_object() {
                    let patch: serde_json::Map<String, Value> = map
                        .iter()
                        .filter(|(key, _)| !PRIVATE.contains(&key.as_str()))
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect();
                    let _ = state::update_settings(&Value::Object(patch));
                }
            }
            let mine: serde_json::Map<String, Value> = state::settings_snapshot()
                .as_object()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter(|(key, _)| !PRIVATE.contains(&key.as_str()))
                .collect();
            let bookmarks = state::with_store(|store| store.bookmarks(5000)).unwrap_or_default();
            let out = json!({"app": "AgentGo", "version": 1, "savedAt": now, "settings": mine, "bookmarks": bookmarks});
            std::fs::write(&file, serde_json::to_vec_pretty(&out).unwrap_or_default())
                .map_err(|error| Fail::internal(error.to_string()))?;
            state::update_settings(&json!({"syncFolder": folder, "syncLast": now}))
                .map_err(Fail::internal)?;
            Ok(
                json!({"syncing": true, "folder": folder, "file": file.display().to_string(), "bookmarksReceived": added, "bookmarksSent": bookmarks.len()}),
            )
        }
    }
}

fn apps_folder() -> std::path::PathBuf {
    std::path::PathBuf::from(std::env::var("APPDATA").unwrap_or_default())
        .join("Microsoft\\Windows\\Start Menu\\Programs\\AgentGo Apps")
}

/// `web_app.install {url, name}` / `web_app.uninstall {name}`: a Start
/// menu entry that opens the site in AgentGo, after the person's approval.
pub fn web_app(caller: &Caller, method: &str, params: &Value) -> Reply {
    let name: String = params["name"]
        .as_str()
        .unwrap_or("")
        .chars()
        .filter(|c| c.is_alphanumeric() || " -_".contains(*c))
        .take(60)
        .collect();
    if name.trim().is_empty() {
        return Err(Fail::params(
            "params.name: the name of the Start menu entry",
        ));
    }
    let link = apps_folder().join(format!("{}.lnk", name.trim()));
    if method == "web_app.uninstall" {
        let removed = std::fs::remove_file(&link).is_ok();
        return Ok(json!({"name": name, "removed": removed}));
    }
    let url = params["url"]
        .as_str()
        .filter(|url| url.starts_with("https://") || url.starts_with("http://"))
        .ok_or_else(|| Fail::params("params.url must be an http or https address"))?;
    let tab = page::tab_of(caller, params)?;
    sessions::require_approval(
        caller,
        &tab,
        "agentgo://outside",
        &format!("add {name} to the Start menu"),
        "add a Start menu entry",
        params["approvalId"].as_str(),
    )?;
    std::fs::create_dir_all(apps_folder()).map_err(|error| Fail::internal(error.to_string()))?;
    let exe = std::env::current_exe().map_err(|error| Fail::internal(error.to_string()))?;
    let script = format!(
        "$l=(New-Object -ComObject WScript.Shell).CreateShortcut('{}');$l.TargetPath='{}';$l.Arguments='run --url \"{}\"';$l.Save()",
        link.display().to_string().replace('\'', "''"),
        exe.display().to_string().replace('\'', "''"),
        url.replace('\'', "''").replace('"', "")
    );
    let status = super::exec::hidden_command("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .status()
        .map_err(|error| Fail::internal(error.to_string()))?;
    if !status.success() || !link.exists() {
        return Err(Fail::internal("the Start menu entry could not be created"));
    }
    Ok(json!({"name": name, "url": url, "shortcut": link.display().to_string()}))
}
