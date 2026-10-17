//! `agentgo import`: brings bookmarks, history, saved passwords and open tabs
//! from another browser on this PC into the AgentGo profile, without starting
//! the browser (the installer uses it). Writes the same stores the browser's
//! own first-run import writes.

use std::path::PathBuf;

use agentgo_import::{Browser, Options, Profile};
use agentgo_storage::{AgentGoPaths, SqliteStateStore};
use serde_json::{json, Value};

const DEFAULT_PROFILE_ID: &str = "default";
/// The sign-in vault holds at most this many sign-ins (one per site).
const VAULT_LIMIT: usize = 5000;

fn name_of(browser: Browser) -> &'static str {
    match browser {
        Browser::Chrome => "Chrome",
        Browser::Edge => "Edge",
        Browser::Brave => "Brave",
        Browser::Firefox => "Firefox",
    }
}

fn paths() -> Result<AgentGoPaths, String> {
    match std::env::var_os("AGENTGO_DATA_ROOT") {
        Some(root) => Ok(AgentGoPaths::with_root(root)),
        None => AgentGoPaths::system_default().map_err(|e| e.to_string()),
    }
}

/// The Chromium root of the person's AgentGo profile (where the vault and
/// the session list live), as the browser picks it at start.
fn profile_root(paths: &AgentGoPaths) -> PathBuf {
    let profile = std::fs::read(paths.config_dir().join("settings.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|settings| settings["profileId"].as_str().map(str::to_string))
        .filter(|id| !id.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_PROFILE_ID.to_string());
    paths.profile_dir(&profile).join("cef")
}

fn scan() -> Value {
    let profiles: Vec<Value> = agentgo_import::detect()
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
                    "passwords": counts.logins,
                    "tabs": counts.tabs,
                },
            })
        })
        .collect();
    json!({ "profiles": profiles })
}

fn find(browser: &str, id: &str) -> Option<Profile> {
    agentgo_import::detect()
        .into_iter()
        .find(|p| name_of(p.browser).eq_ignore_ascii_case(browser) && p.id == id)
}

fn browser_running() -> bool {
    // The browser holds this lock for the whole run.
    let Ok(paths) = paths() else { return false };
    let lock = profile_root(&paths).join("agentgo.lock");
    lock.exists() && std::fs::OpenOptions::new().write(true).open(&lock).is_err()
}

fn import(browser: &str, id: &str, what: &str) -> Result<Value, String> {
    let want = |key: &str| what.split(',').any(|w| w.trim().eq_ignore_ascii_case(key));
    let profile =
        find(browser, id).ok_or_else(|| format!("no {browser} profile \"{id}\" was found"))?;
    if browser_running() {
        return Err("AgentGo is open. Close it, then import again.".into());
    }
    let options = Options {
        bookmarks: want("bookmarks"),
        history: want("history"),
        logins: want("passwords") || want("logins"),
        tabs: want("tabs"),
        ..Options::default()
    };
    let imported = agentgo_import::import(&profile, &options)?;
    let paths = paths()?;
    let mut notes = imported.notes.clone();

    let (mut bookmarks, mut history) = (0usize, 0usize);
    if !imported.bookmarks.is_empty() || !imported.history.is_empty() {
        std::fs::create_dir_all(paths.data_dir()).map_err(|e| e.to_string())?;
        let store =
            SqliteStateStore::open(paths.data_dir().join("state.db")).map_err(|e| e.to_string())?;
        for item in &imported.bookmarks {
            if store
                .add_bookmark(&item.url, &item.title, (item.added_ms.max(0) / 1000) as u64)
                .is_ok()
            {
                bookmarks += 1;
            }
        }
        for item in imported.history.iter().rev() {
            let title = if item.title.is_empty() {
                &item.url
            } else {
                &item.title
            };
            if store
                .record_visit(&item.url, title, (item.last_visit_ms.max(0) / 1000) as u64)
                .is_ok()
            {
                history += 1;
            }
        }
    }

    let root = profile_root(&paths);
    let pairs: Vec<(String, String, String)> = imported
        .logins
        .iter()
        .map(|l| (l.origin.clone(), l.username.clone(), l.password.clone()))
        .collect();
    let passwords = save_logins(&root, pairs, &mut notes)?;

    let mut tabs = 0usize;
    if !imported.tabs.is_empty() {
        // The browser reopens session.json at its next start.
        let file = root.join("session.json");
        let mut session: Vec<String> = std::fs::read(&file)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        for url in imported.tabs.iter().take(20) {
            if !session.contains(url) {
                session.push(url.clone());
                tabs += 1;
            }
        }
        std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
        std::fs::write(
            &file,
            serde_json::to_vec(&session).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    }

    Ok(json!({
        "imported": {"bookmarks": bookmarks, "history": history, "passwords": passwords, "tabs": tabs},
        "notes": notes,
    }))
}

/// Adds sign-ins to the profile's vault (one per site); returns how many.
fn save_logins(
    root: &std::path::Path,
    logins: Vec<(String, String, String)>,
    notes: &mut Vec<String>,
) -> Result<usize, String> {
    if logins.is_empty() {
        return Ok(0);
    }
    let store = agentgo_accounts::LoginStore::new(
        root.join("accounts").join("logins.bin"),
        agentgo_accounts::Dpapi,
    );
    let mut all = store.load().map_err(|e| e.to_string())?;
    let (mut saved, mut skipped) = (0usize, 0usize);
    for (origin, username, password) in logins {
        let Ok(login) = agentgo_accounts::Login::new(&origin, &username, &password) else {
            skipped += 1;
            continue;
        };
        all.retain(|known| known.origin != login.origin);
        if all.len() >= VAULT_LIMIT {
            skipped += 1;
            continue;
        }
        all.push(login);
        saved += 1;
    }
    store.save(&all).map_err(|e| e.to_string())?;
    if skipped > 0 {
        notes.push(format!(
            "{skipped} saved passwords were skipped (one sign-in per site, up to {VAULT_LIMIT})."
        ));
    }
    Ok(saved)
}

fn import_csv(file: &std::path::Path) -> Result<Value, String> {
    if browser_running() {
        return Err("AgentGo is open. Close it, then import again.".into());
    }
    let size = std::fs::metadata(file).map_err(|e| e.to_string())?.len();
    if size > 5 * 1024 * 1024 {
        return Err("that file is too large to be a password export".into());
    }
    let text = std::fs::read_to_string(file).map_err(|e| e.to_string())?;
    let parsed = agentgo_import::parse_password_csv(&text);
    let mut notes = Vec::new();
    if parsed.is_empty() {
        notes.push(
            "No sign-ins were found. The file needs url, username and password columns."
                .to_string(),
        );
    }
    let root = profile_root(&paths()?);
    let saved = save_logins(
        &root,
        parsed
            .into_iter()
            .map(|l| (l.origin, l.username, l.password))
            .collect(),
        &mut notes,
    )?;
    Ok(json!({"imported": {"passwords": saved}, "notes": notes}))
}

fn open_export(browser: &str) -> Result<Value, String> {
    let (program, page) = match browser.to_ascii_lowercase().as_str() {
        "chrome" => ("chrome", "chrome://password-manager/settings"),
        "edge" => ("msedge", "edge://password-manager/settings"),
        "brave" => ("brave", "brave://password-manager/settings"),
        "firefox" => ("firefox", "about:logins"),
        _ => return Err("unknown browser".into()),
    };
    let mut command = std::process::Command::new("cmd");
    command.args(["/C", "start", "", program, page]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    command.spawn().map_err(|e| e.to_string())?;
    Ok(json!({"opened": page}))
}

/// Entry point for `agentgo import`. Prints JSON; returns the exit code.
pub fn run(
    browser: Option<&str>,
    id: Option<&str>,
    what: &str,
    csv: Option<&std::path::Path>,
    open: Option<&str>,
) -> i32 {
    let result = match (open, csv, browser) {
        (Some(browser), _, _) => open_export(browser),
        (None, Some(file), _) => import_csv(file),
        (None, None, None) => Ok(scan()),
        (None, None, Some(browser)) => import(browser, id.unwrap_or("Default"), what),
    };
    match result {
        Ok(value) => {
            println!("{value}");
            0
        }
        Err(error) => {
            println!("{}", json!({"error": error}));
            1
        }
    }
}
