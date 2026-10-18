//! First-run setup: importing from another browser and connecting an agent
//! host. Reachable only from the trusted tools document.

use agentgo_import::{Browser, Options, Profile};
use serde_json::{json, Value};

use super::sessions::{Fail, Reply};
use super::{accounts, exec, state};

fn name_of(browser: Browser) -> &'static str {
    match browser {
        Browser::Chrome => "Chrome",
        Browser::Edge => "Edge",
        Browser::Brave => "Brave",
        Browser::Firefox => "Firefox",
    }
}

fn find(browser: &str, id: &str) -> Option<Profile> {
    agentgo_import::detect()
        .into_iter()
        .find(|profile| name_of(profile.browser) == browser && profile.id == id)
}

/// `ui.import_scan`: the browsers found on this computer and what each holds.
pub async fn scan() -> Reply {
    let profiles = exec::blocking(20_000, || {
        agentgo_import::detect()
            .into_iter()
            .map(|profile| {
                let counts = agentgo_import::count(&profile);
                json!({
                    "browser": name_of(profile.browser),
                    "id": profile.id,
                    "name": profile.name,
                    "counts": {
                        "bookmarks": counts.bookmarks,
                        "history": counts.history,
                        "logins": counts.logins,
                        "tabs": counts.tabs,
                        "extensions": counts.extensions,
                    },
                })
            })
            .collect::<Vec<_>>()
    })
    .await
    .ok_or_else(|| Fail::internal("looking for other browsers timed out"))?;
    Ok(json!({"profiles": profiles}))
}

/// `ui.import_run`: copies the chosen data into this profile, or, with
/// `{connect}`, registers the browser with an agent host.
pub async fn run(params: &Value) -> Reply {
    if let Some(host) = params["connect"].as_str() {
        return connect(host).await;
    }
    if let Some(csv) = params["csv"].as_str() {
        return import_csv(csv);
    }
    if let Some(browser) = params["openExport"].as_str() {
        return open_export(browser);
    }
    let browser = params["browser"].as_str().unwrap_or("");
    let id = params["id"].as_str().unwrap_or("");
    let profile = find(browser, id).ok_or_else(|| Fail::params("that browser profile is gone"))?;
    let flag = |key: &str| params["options"][key] == true;
    let options = Options {
        bookmarks: flag("bookmarks"),
        history: flag("history"),
        logins: flag("logins"),
        tabs: flag("tabs"),
        extensions: flag("extensions"),
        ..Options::default()
    };
    let imported = exec::blocking(120_000, move || {
        let imported = agentgo_import::import(&profile, &options)?;
        let mut stored = (0usize, 0usize);
        for item in &imported.bookmarks {
            let added = (item.added_ms.max(0) / 1000) as u64;
            if state::with_store(|store| store.add_bookmark(&item.url, &item.title, added)).is_ok()
            {
                stored.0 += 1;
            }
        }
        for item in imported.history.iter().rev() {
            let seen = (item.last_visit_ms.max(0) / 1000) as u64;
            let title = if item.title.is_empty() {
                &item.url
            } else {
                &item.title
            };
            if state::with_store(|store| store.record_visit(&item.url, title, seen)).is_ok() {
                stored.1 += 1;
            }
        }
        Ok::<_, String>((imported, stored))
    })
    .await
    .ok_or_else(|| Fail::internal("the import took too long"))?
    .map_err(Fail::internal)?;
    let (imported, (bookmarks, history)) = imported;
    let mut notes = imported.notes.clone();
    let logins = accounts::import_logins(
        imported
            .logins
            .iter()
            .map(|login| {
                (
                    login.origin.clone(),
                    login.username.clone(),
                    login.password.clone(),
                )
            })
            .collect(),
    );
    if logins.skipped > 0 {
        notes.push(format!(
            "{} saved passwords were skipped (the vault keeps one sign-in per site, up to 5000).",
            logins.skipped
        ));
    }
    state::reload_history();
    state::notify_shell_state();
    let mut opened = 0;
    for url in imported.tabs.iter().take(20) {
        if state::create_content_tab(url).is_ok() {
            opened += 1;
        }
    }
    // Extensions come fresh from the Web Store, checked against their
    // signature; only the ids were read from the other browser.
    let mut extensions = Vec::new();
    let mut unsupported = Vec::new();
    for extension in imported.extensions.iter().take(40) {
        match super::extensions::install_from_store(&extension.id).await {
            Ok(_) => extensions.push(extension.name.clone()),
            Err(fail) => {
                let reason = fail
                    .message
                    .rsplit(": ")
                    .next()
                    .unwrap_or(&fail.message)
                    .to_string();
                unsupported.push(json!({"name": extension.name, "reason": reason}));
            }
        }
    }
    if !unsupported.is_empty() {
        notes.push(format!(
            "{} extension(s) could not be brought over: {}.",
            unsupported.len(),
            unsupported
                .iter()
                .map(|u| format!(
                    "{} ({})",
                    u["name"].as_str().unwrap_or(""),
                    u["reason"].as_str().unwrap_or("")
                ))
                .collect::<Vec<_>>()
                .join("; ")
        ));
    }
    if !extensions.is_empty() {
        state::run_in_shell("typeof refreshExtensions === 'function' && refreshExtensions()");
    }
    Ok(json!({
        "imported": {"bookmarks": bookmarks, "history": history, "logins": logins.saved, "tabs": opened, "extensions": extensions.len()},
        "extensions": extensions,
        "unsupportedExtensions": unsupported,
        "notes": notes,
    }))
}

/// Opens the other browser on its own password export page. Passwords that
/// browser protects with app-bound encryption can only leave through its
/// export, never by reading its files.
fn open_export(browser: &str) -> Reply {
    let (program, page) = match browser {
        "Chrome" => ("chrome", "chrome://password-manager/settings"),
        "Edge" => ("msedge", "edge://password-manager/settings"),
        "Brave" => ("brave", "brave://password-manager/settings"),
        "Firefox" => ("firefox", "about:logins"),
        _ => return Err(Fail::params("unknown browser")),
    };
    // `start` finds the browser through its App Paths registration.
    exec::hidden_command("cmd")
        .args(["/C", "start", "", program, page])
        .spawn()
        .map_err(|error| Fail::internal(error.to_string()))?;
    Ok(json!({"opened": page}))
}

/// Imports a password-manager CSV export into the sign-in vault.
fn import_csv(csv: &str) -> Reply {
    if csv.len() > 5 * 1024 * 1024 {
        return Err(Fail::params(
            "that file is too large to be a password export",
        ));
    }
    let parsed = agentgo_import::parse_password_csv(csv);
    if parsed.is_empty() {
        return Ok(json!({
            "imported": {"logins": 0},
            "notes": ["No sign-ins were found. The file needs url, username and password columns."],
        }));
    }
    let logins = accounts::import_logins(
        parsed
            .into_iter()
            .map(|login| (login.origin, login.username, login.password))
            .collect(),
    );
    let mut notes = Vec::new();
    if logins.skipped > 0 {
        notes.push(format!(
            "{} sign-ins were skipped (the vault keeps one sign-in per site, up to 5000).",
            logins.skipped
        ));
    }
    state::notify_shell_state();
    Ok(json!({"imported": {"logins": logins.saved}, "notes": notes}))
}

/// Registers AgentGo with an agent host through `agentgo-tool setup <host>`.
async fn connect(host: &str) -> Reply {
    if !["claude", "codex", "droid", "json"].contains(&host) {
        return Err(Fail::params("unknown agent host"));
    }
    let tool = std::env::current_exe()
        .map_err(|e| Fail::internal(e.to_string()))?
        .with_file_name("agentgo-tool.exe");
    let host = host.to_string();
    let result = exec::blocking(30_000, move || {
        let output = exec::hidden_command(&tool.to_string_lossy())
            .args(["setup", &host])
            .output()
            .map_err(|error| error.to_string())?;
        Ok::<_, String>((
            output.status.success(),
            String::from_utf8_lossy(&output.stdout).trim().to_string(),
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
            host,
        ))
    })
    .await
    .ok_or_else(|| Fail::internal("the agent host did not answer"))?
    .map_err(Fail::internal)?;
    let (ok, out, err, host) = result;
    if host == "json" {
        return Ok(json!({"ok": ok, "config": out, "text": "Config ready"}));
    }
    Ok(json!({
        "ok": ok,
        "text": if ok {
            format!("Connected to {host}")
        } else if err.is_empty() {
            format!("Could not reach {host}. Is it installed?")
        } else {
            err.lines().next().unwrap_or("").to_string()
        },
    }))
}

/// `ui.setup_done`: the wizard is not shown again.
pub fn done() -> Reply {
    state::update_settings(&json!({"setupDone": true})).map_err(Fail::params)?;
    Ok(json!({"done": true}))
}
