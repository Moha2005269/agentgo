//! The browser process itself: other instances (`launch`, `attach`,
//! `remote_instance`), `restart`, `recover`, `suspend`/`resume`, low power
//! and a tab budget, the command line and feature flags, the UI loop's
//! health, and auto-update.
//!
//! Auto-update: the person sets a manifest address (setting
//! `updateManifest`): JSON `{version, url, sha256, notes}`. `update_check`
//! reads it; `update_stage` downloads the zip, refuses it unless its SHA-256
//! matches, unpacks it beside the install (`updates/<version>`) and, when
//! asked, swaps it in after AgentGo exits (`update.ps1`), keeping the old
//! version in `updates/previous` for `update_rollback`.

use std::cell::RefCell;
use std::path::PathBuf;

use agentgo_protocol::ErrorCode;
use serde_json::{json, Value};

use super::sessions::{self, Caller, Control, Fail, Reply};
use super::{cdp, dispatch, exec, state};

const SWAP: &str = include_str!("update.ps1");

thread_local! {
    static BUDGET: RefCell<Option<usize>> = const { RefCell::new(None) };
    static STARTED: std::time::Instant = std::time::Instant::now();
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

/// Runs a program without a console window and returns its output.
async fn run(
    program: &str,
    args: Vec<String>,
    timeout_ms: u64,
) -> Result<std::process::Output, Fail> {
    let name = program.to_string();
    let program = name.clone();
    exec::blocking(timeout_ms, move || {
        #[cfg(windows)]
        use std::os::windows::process::CommandExt;
        let mut command = std::process::Command::new(&program);
        command.args(&args).stdin(std::process::Stdio::null());
        #[cfg(windows)]
        command.creation_flags(0x0800_0000);
        command.output().map_err(|e| e.to_string())
    })
    .await
    .ok_or_else(|| Fail::new(ErrorCode::Timeout, format!("{name} did not finish in time")))?
    .map_err(Fail::internal)
}

/// Starts a program that outlives AgentGo (no window, own process group).
fn spawn_detached(program: &str, args: &[String], env: &[(&str, &str)]) -> Result<u32, Fail> {
    #[cfg(windows)]
    use std::os::windows::process::CommandExt;
    let mut command = std::process::Command::new(program);
    command
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    for (key, value) in env {
        if value.is_empty() {
            command.env_remove(key);
        } else {
            command.env(key, value);
        }
    }
    #[cfg(windows)]
    // No window and its own process group. DETACHED_PROCESS is not used: it
    // overrides CREATE_NO_WINDOW and a console program then opens a window.
    command.creation_flags(0x0800_0000 | 0x0000_0200);
    command
        .spawn()
        .map(|child| child.id())
        .map_err(|e| Fail::internal(e.to_string()))
}

fn instances_dir() -> PathBuf {
    state::data_dir()
        .parent()
        .map(|root| root.join("instances"))
        .unwrap_or_default()
}

fn running() -> Vec<Value> {
    let own = &state::state().instance_id;
    let mut out = Vec::new();
    for entry in std::fs::read_dir(instances_dir())
        .into_iter()
        .flatten()
        .flatten()
    {
        let token = entry.path().join("token");
        if token.exists() {
            let id = entry.file_name().to_string_lossy().to_string();
            out.push(json!({"instanceId": id, "pipe": super::pipe::pipe_name(&id), "tokenFile": token.display().to_string(), "self": &id == own}));
        }
    }
    out
}

fn install_dir() -> PathBuf {
    std::env::var_os("AGENTGO_INSTALL_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::current_exe()
                .ok()
                .and_then(|exe| exe.parent().map(PathBuf::from))
        })
        .unwrap_or_default()
}

thread_local! {
    /// What the automatic update knows: {available, staged, notes, error}.
    static UPDATE: RefCell<Value> = RefCell::new(json!({}));
}

fn set_update_state(value: Value) {
    UPDATE.with(|u| *u.borrow_mut() = value.clone());
    let script = format!("window.__agentgoUpdate && window.__agentgoUpdate({value});");
    state::run_in_shell(&script);
}

/// Only an installed, packaged copy replaces its own files: never a
/// development build or a test run.
fn updates_itself() -> bool {
    if std::env::var_os("AGENTGO_UPDATE_TEST").is_some() {
        return true;
    }
    let options = &state::state().options;
    !cfg!(debug_assertions)
        && !options.stdio
        && !options.smoke
        && install_dir().join("SHA256SUMS.txt").is_file()
}

/// Checks for a new release shortly after start and every six hours. With
/// automatic updates on, a new release is downloaded and checked in the
/// background and swapped in when AgentGo closes; otherwise the toolbar
/// offers it.
pub fn start_update_checks() {
    if !updates_itself() {
        return;
    }
    // Tests shorten the wait before the first check.
    let first = std::env::var("AGENTGO_UPDATE_DELAY_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|_| std::env::var_os("AGENTGO_UPDATE_TEST").is_some())
        .unwrap_or(20_000);
    exec::spawn(async move {
        exec::sleep(first).await;
        loop {
            check_and_stage().await;
            exec::sleep(6 * 60 * 60 * 1000).await;
        }
    });
}

async fn check_and_stage() {
    let found = match manifest(&json!({})).await {
        Ok(found) => found,
        Err(fail) => {
            state::log_event(&format!("AG-UPDATE check-failed {}", fail.message));
            return;
        }
    };
    let latest = found["version"].as_str().unwrap_or("").to_string();
    if !newer(&latest, env!("CARGO_PKG_VERSION")) {
        return;
    }
    state::log_event(&format!("AG-UPDATE available {latest}"));
    set_update_state(json!({"available": latest, "staged": false, "notes": found["notes"]}));
    if state::settings_snapshot()["autoUpdate"] == false {
        return;
    }
    let shell = Caller::shell();
    match command(&shell, "browser.update_stage", &json!({"apply": "on_exit"})).await {
        Ok(_) => state::log_event(&format!(
            "AG-UPDATE staged {latest} (swapped in when AgentGo closes)"
        )),
        Err(fail) => {
            state::log_event(&format!("AG-UPDATE stage-failed {}", fail.message));
            set_update_state(
                json!({"available": latest, "staged": false, "error": fail.message, "notes": found["notes"]}),
            );
        }
    }
}

/// `major.minor.patch` (missing parts are 0) and whether it is a final
/// release: `0.2.0-rc1` comes before `0.2.0`.
fn version_parts(text: &str) -> ([u64; 3], bool) {
    let text = text.trim().trim_start_matches('v');
    let (core, release) = match text.split_once('-') {
        Some((core, _)) => (core, false),
        None => (text, true),
    };
    let mut parts = [0u64; 3];
    for (slot, part) in parts.iter_mut().zip(core.split('.')) {
        *slot = part.parse().unwrap_or(0);
    }
    (parts, release)
}

fn newer(candidate: &str, current: &str) -> bool {
    version_parts(candidate) > version_parts(current)
}

async fn fetch(url: &str, out: Option<&std::path::Path>) -> Result<Vec<u8>, Fail> {
    if !(url.starts_with("https://")
        || url.starts_with("http://127.0.0.1")
        || url.starts_with("http://localhost"))
    {
        return Err(Fail::params(
            "updates come over https (or from this computer for testing)",
        ));
    }
    let mut args = vec!["-sSfL".to_string(), "--max-time".into(), "600".into()];
    if let Some(path) = out {
        args.push("-o".into());
        args.push(path.display().to_string());
    }
    args.push(url.to_string());
    let done = run("curl.exe", args, 620_000).await?;
    if !done.status.success() {
        return Err(Fail::internal(format!(
            "could not download {url}: {}",
            String::from_utf8_lossy(&done.stderr).trim()
        )));
    }
    Ok(done.stdout)
}

/// Ed25519 public key (hex) whose private half signs every release manifest.
const UPDATE_KEY: &str = include_str!("update-key.pub");

/// The exact text a release signature covers. Keep in step with
/// `scripts/dev/sign-update.js`.
fn signed_text(manifest: &Value) -> String {
    format!(
        "agentgo-update-v1\n{}\n{}\n{}\n",
        manifest["version"].as_str().unwrap_or(""),
        manifest["url"].as_str().unwrap_or(""),
        manifest["sha256"]
            .as_str()
            .unwrap_or("")
            .to_ascii_lowercase()
    )
}

/// The release key. Debug builds (never the shipped one) let tests substitute
/// their own with `AGENTGO_UPDATE_PUBKEY`.
fn update_key() -> String {
    if cfg!(debug_assertions) {
        if let Ok(key) = std::env::var("AGENTGO_UPDATE_PUBKEY") {
            return key;
        }
    }
    UPDATE_KEY.to_string()
}

fn from_hex(text: &str) -> Option<Vec<u8>> {
    let text = text.trim();
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
        .collect()
}

fn signature_valid(manifest: &Value, key_hex: &str) -> bool {
    let (Some(key), Some(signature)) = (
        from_hex(key_hex),
        manifest["signature"].as_str().and_then(from_hex),
    ) else {
        return false;
    };
    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, key)
        .verify(signed_text(manifest).as_bytes(), &signature)
        .is_ok()
}

/// Where releases publish their signed manifest.
const DEFAULT_MANIFEST: &str =
    "https://github.com/Moha2005269/agentgo/releases/latest/download/update-manifest.json";

async fn manifest(params: &Value) -> Result<Value, Fail> {
    let url = params["manifest"]
        .as_str()
        .map(str::to_string)
        .or_else(|| {
            state::settings_snapshot()["updateManifest"]
                .as_str()
                .filter(|u| !u.trim().is_empty())
                .map(str::to_string)
        })
        .unwrap_or_else(|| DEFAULT_MANIFEST.to_string());
    let body = fetch(&url, None).await?;
    let value: Value = serde_json::from_slice(&body)
        .map_err(|e| Fail::params(format!("the manifest is not JSON: {e}")))?;
    for key in ["version", "url", "sha256"] {
        if !value[key].is_string() {
            return Err(Fail::params(format!("the manifest has no {key}")));
        }
    }
    if !signature_valid(&value, &update_key()) {
        return Err(Fail::new(
            ErrorCode::PolicyDenied,
            "the update manifest is not signed by AgentGo's release key; it was ignored",
        ));
    }
    Ok(value)
}

/// The release zip holds one folder (`AgentGo-x.y.z-win64\`); the swap needs
/// the program files directly in the staging folder.
fn lift_single_root(staged: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(staged) else {
        return;
    };
    let entries: Vec<_> = entries.flatten().collect();
    let [only] = entries.as_slice() else {
        return;
    };
    if !only.path().is_dir() {
        return;
    }
    let inner = only.path();
    let Ok(children) = std::fs::read_dir(&inner) else {
        return;
    };
    for child in children.flatten() {
        let _ = std::fs::rename(child.path(), staged.join(child.file_name()));
    }
    let _ = std::fs::remove_dir(&inner);
}

fn swap_script() -> Result<PathBuf, Fail> {
    let path =
        std::env::temp_dir().join(format!("agentgo-update-{}.ps1", env!("CARGO_PKG_VERSION")));
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice(SWAP.as_bytes());
    std::fs::write(&path, bytes).map_err(|e| Fail::internal(e.to_string()))?;
    Ok(path)
}

/// Starts the swap after this process exits; `relaunch` starts AgentGo again.
fn schedule_swap(
    staged: &std::path::Path,
    backup: &std::path::Path,
    relaunch: bool,
) -> Result<(), Fail> {
    // Never replace a folder that holds the person's data (profiles, saved
    // sign-ins, accounts, history, settings): those live apart from the
    // program, in AgentGo's data folder, and an update never touches them.
    let data = state::data_dir();
    let data_root = data.parent().unwrap_or(&data);
    let install = install_dir();
    if data_root.starts_with(&install) || install.starts_with(data_root) {
        return Err(Fail::new(
            ErrorCode::PolicyDenied,
            "the install folder holds AgentGo's profile data; the update was not applied",
        ));
    }
    // An installed copy updates through AgentGo Setup, which shows the
    // update (same film as installing); a portable copy swaps quietly.
    let setup = install.join("AgentGo-Setup.exe");
    if setup.is_file() {
        let mut args = vec![
            "--update".to_string(),
            "--install".into(),
            install.display().to_string(),
            "--staged".into(),
            staged.display().to_string(),
            "--backup".into(),
            backup.display().to_string(),
            "--wait-pid".into(),
            std::process::id().to_string(),
        ];
        if relaunch {
            args.push("--relaunch".into());
        }
        spawn_detached(&setup.display().to_string(), &args, &[])?;
        return Ok(());
    }
    let script = swap_script()?;
    let mut args = vec![
        "-NoProfile".to_string(),
        "-NonInteractive".into(),
        "-ExecutionPolicy".into(),
        "Bypass".into(),
        "-File".into(),
        script.display().to_string(),
        "-WaitPid".into(),
        std::process::id().to_string(),
        "-Install".into(),
        install_dir().display().to_string(),
        "-Staged".into(),
        staged.display().to_string(),
        "-Backup".into(),
        backup.display().to_string(),
    ];
    if relaunch {
        args.push("-Relaunch".into());
        args.push("run --restore".into());
    }
    spawn_detached("powershell", &args, &[])?;
    Ok(())
}

/// Before a new tab: with a tab budget, the oldest background tab nobody
/// drives is discarded to make room.
pub async fn make_room(caller: &Caller) {
    let Some(max) = BUDGET.with(|b| *b.borrow()) else {
        return;
    };
    let tabs = state::content_tab_ids();
    if tabs.len() < max {
        return;
    }
    let active = state::active_tab_id();
    for tab in tabs {
        if tab != active
            && sessions::lease_holder(&tab).is_none()
            && state::tab_url(&tab) != "about:blank"
        {
            let _ = Box::pin(dispatch::run(
                &Caller::shell(),
                "tab.discard",
                json!({"tabId": tab}),
            ))
            .await;
            state::log_event(&format!(
                "AG-RUNTIME budget-discarded {tab} for {}",
                caller.session
            ));
            return;
        }
    }
}

async fn freeze_all(on: bool, background_only: bool) -> Vec<String> {
    let active = state::active_tab_id();
    let mut changed = Vec::new();
    for tab in state::content_tab_ids() {
        if background_only && (tab == active || sessions::lease_holder(&tab).is_some()) {
            continue;
        }
        let state_name = if on { "frozen" } else { "active" };
        if cdp::call(
            &tab,
            "Page.setWebLifecycleState",
            json!({"state": state_name}),
        )
        .await
        .is_ok()
        {
            changed.push(tab);
        }
    }
    changed
}

pub fn handles(method: &str) -> bool {
    matches!(
        method,
        "browser.launch"
            | "browser.attach"
            | "browser.detach"
            | "browser.restart"
            | "browser.feature_flags"
            | "browser.command_line"
            | "browser.message_loop_status"
            | "browser.recover"
            | "browser.suspend"
            | "browser.resume"
            | "browser.low_power_mode"
            | "browser.resource_budget"
            | "browser.remote_instance"
            | "browser.update_check"
            | "browser.update_status"
            | "browser.update_stage"
            | "browser.update_rollback"
            | "security.status"
    )
}

pub async fn command(caller: &Caller, method: &str, params: &Value) -> Reply {
    match method {
        "browser.launch" => {
            let exe = std::env::current_exe().map_err(|e| Fail::internal(e.to_string()))?;
            let mut args = vec!["run".to_string()];
            if let Some(url) = params["url"]
                .as_str()
                .filter(|u| state::is_navigable_url(u))
            {
                args.push("--url".into());
                args.push(url.to_string());
            }
            if let Some(profile) = params["profile"].as_str() {
                args.push("--profile".into());
                args.push(
                    profile
                        .chars()
                        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                        .take(32)
                        .collect(),
                );
            }
            if params["ephemeral"] == true {
                args.push("--ephemeral".into());
            }
            let before: Vec<String> = running()
                .iter()
                .filter_map(|i| i["instanceId"].as_str().map(str::to_string))
                .collect();
            let root = state::data_dir()
                .parent()
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            // An agent's instance opens behind the person's windows.
            let pid = spawn_detached(
                &exe.display().to_string(),
                &args,
                &[
                    ("AGENTGO_DATA_ROOT", root.as_str()),
                    ("AGENTGO_BACKGROUND", "1"),
                    // Start the browser itself, not a launcher that starts it
                    // again without a console and exits: the pid reported
                    // below must be the browser's.
                    ("AGENTGO_NO_CONSOLE", "1"),
                    ("AGENTGO_REMOTE_DEBUGGING_PORT", ""),
                    ("AGENTGO_UNSAFE_DEVTOOLS", ""),
                ],
            )?;
            let started = std::time::Instant::now();
            loop {
                if let Some(new) = running().into_iter().find(|i| {
                    i["instanceId"]
                        .as_str()
                        .is_some_and(|id| !before.iter().any(|b| b == id))
                }) {
                    let mut out = new;
                    if !out["pid"].is_u64() {
                        out["pid"] = json!(pid);
                    }
                    out["attach"] = json!(
                        "connect to the pipe and send session.hello with the token from tokenFile"
                    );
                    return Ok(out);
                }
                if started.elapsed().as_secs() > 40 {
                    return Err(Fail::new(
                        ErrorCode::Timeout,
                        "the new instance did not start in time",
                    )
                    .with(json!({"pid": pid})));
                }
                exec::sleep(250).await;
            }
        }
        "browser.remote_instance" => Ok(json!({"instances": running()})),
        "browser.attach" => {
            let wanted = params["instanceId"]
                .as_str()
                .unwrap_or(&state::state().instance_id)
                .to_string();
            running().into_iter().find(|i| i["instanceId"] == wanted.as_str()).map(|mut i| {
                i["hello"] = json!({"method": "session.hello", "params": {"token": "<the content of tokenFile>", "client": "your name"}});
                i
            }).ok_or_else(|| Fail::params(format!("no running instance {wanted}")))
        }
        "browser.detach" => {
            let released = sessions::release_all(&caller.session);
            Ok(
                json!({"released": released, "note": "the tabs stay open for the person; the session stays connected"}),
            )
        }
        "browser.restart" => {
            operator(caller, "restarting AgentGo")?;
            if state::state().options.stdio {
                return Err(Fail::params("this AgentGo was started by an agent over stdio; it restarts by being started again"));
            }
            let exe = std::env::current_exe().map_err(|e| Fail::internal(e.to_string()))?;
            // The windowless launcher, so the new instance opens no console.
            let launcher = exe.with_file_name("agentgo-browser.exe");
            let exe = if launcher.exists() { launcher } else { exe };
            let mut args: Vec<String> = std::env::args().skip(1).collect();
            if !args.iter().any(|a| a == "--restore") {
                args.push("--restore".into());
            }
            let quoted: Vec<String> = args
                .iter()
                .map(|a| format!("'{}'", a.replace('\'', "''")))
                .collect();
            let script = format!("Wait-Process -Id {} -Timeout 60 -ErrorAction SilentlyContinue; Start-Process -FilePath '{}' -ArgumentList @({})", std::process::id(), exe.display(), quoted.join(","));
            spawn_detached(
                "powershell",
                &[
                    "-NoProfile".into(),
                    "-NonInteractive".into(),
                    "-WindowStyle".into(),
                    "Hidden".into(),
                    "-Command".into(),
                    script,
                ],
                &[],
            )?;
            exec::spawn(async {
                exec::sleep(300).await;
                state::request_quit(0);
            });
            Ok(json!({"restarting": true, "restore": true}))
        }
        "browser.recover" => {
            let mut reloaded = Vec::new();
            let mut stuck = Vec::new();
            for tab in state::tabs_snapshot() {
                let id = tab["tabId"].as_str().unwrap_or("").to_string();
                if !sessions::can_see_tab(caller, &id) {
                    continue;
                }
                if tab["crashed"] == true {
                    if Box::pin(dispatch::run(caller, "tab.reload", json!({"tabId": id})))
                        .await
                        .is_ok()
                    {
                        reloaded.push(id);
                    }
                } else if tab["unresponsive"] == true {
                    stuck.push(id);
                }
            }
            Ok(
                json!({"reloaded": reloaded, "unresponsive": stuck, "next": if stuck.is_empty() { "" } else { "an unresponsive page usually recovers; tab.reload or tabs.close if it does not" }}),
            )
        }
        "browser.suspend" | "browser.resume" => {
            operator(caller, "suspending AgentGo")?;
            let on = method == "browser.suspend";
            sessions::set_control(if on {
                Control::Paused
            } else {
                Control::Running
            });
            let tabs = freeze_all(on, false).await;
            Ok(
                json!({"suspended": on, "tabs": tabs, "agents": if on { "paused" } else { "running" }}),
            )
        }
        "browser.low_power_mode" => {
            let on = params["on"] != false;
            let tabs = freeze_all(on, on).await;
            Ok(
                json!({"lowPower": on, "tabs": tabs, "note": if on { "background tabs nobody drives are frozen; each runs again when shown" } else { "" }}),
            )
        }
        "browser.resource_budget" => {
            if let Some(max) = params["maxTabs"].as_u64() {
                operator(caller, "setting the tab budget")?;
                BUDGET.with(|b| *b.borrow_mut() = (max > 0).then_some(max.clamp(2, 500) as usize));
            }
            let script = format!(
                "$all = Get-CimInstance Win32_Process; $ids = @({pid}); do {{ $n = $ids.Count; $ids += @($all | Where-Object {{ $ids -contains $_.ParentProcessId -and $ids -notcontains $_.ProcessId }} | ForEach-Object ProcessId) }} while ($ids.Count -gt $n); $p = @($all | Where-Object {{ $ids -contains $_.ProcessId }}); @{{ processes = $p.Count; workingSetBytes = ($p | Measure-Object WorkingSetSize -Sum).Sum; privateBytes = ($p | Measure-Object PrivatePageCount -Sum).Sum }} | ConvertTo-Json -Compress",
                pid = std::process::id()
            );
            let out = run(
                "powershell",
                vec![
                    "-NoProfile".into(),
                    "-NonInteractive".into(),
                    "-Command".into(),
                    script,
                ],
                30_000,
            )
            .await?;
            let usage: Value = serde_json::from_slice(&out.stdout).unwrap_or(Value::Null);
            Ok(
                json!({"usage": usage, "tabs": state::content_tab_ids().len(), "maxTabs": BUDGET.with(|b| *b.borrow())}),
            )
        }
        "browser.command_line" => {
            let line = cef::command_line_get_global()
                .map(|line| {
                    use cef::ImplCommandLine;
                    cef::CefString::from(&line.command_line_string()).to_string()
                })
                .unwrap_or_default();
            let switches: Vec<String> = line
                .split_whitespace()
                .filter(|part| part.starts_with("--"))
                .map(str::to_string)
                .collect();
            Ok(json!({"commandLine": line, "switches": switches}))
        }
        "browser.feature_flags" => {
            let file = state::data_dir().join("feature-flags.json");
            let mut flags: Value = std::fs::read(&file)
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
                .unwrap_or_else(|| json!({"enable": [], "disable": []}));
            if params.get("enable").is_some() || params.get("disable").is_some() {
                operator(caller, "changing Chromium feature flags")?;
                for key in ["enable", "disable"] {
                    if let Some(list) = params[key].as_array() {
                        let clean: Vec<String> = list
                            .iter()
                            .filter_map(|f| f.as_str())
                            .filter(|f| {
                                !f.is_empty()
                                    && f.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                            })
                            .map(str::to_string)
                            .collect();
                        flags[key] = json!(clean);
                    }
                }
                let _ =
                    std::fs::write(&file, serde_json::to_vec_pretty(&flags).unwrap_or_default());
            }
            let line = cef::command_line_get_global()
                .map(|line| {
                    use cef::ImplCommandLine;
                    cef::CefString::from(&line.command_line_string()).to_string()
                })
                .unwrap_or_default();
            let running_now = |name: &str| {
                line.split_whitespace()
                    .find_map(|part| part.strip_prefix(&format!("--{name}=")))
                    .map(|v| v.split(',').map(str::to_string).collect::<Vec<_>>())
                    .unwrap_or_default()
            };
            Ok(
                json!({"next": flags, "now": {"enable": running_now("enable-features"), "disable": running_now("disable-features")}, "note": "changes apply the next time AgentGo starts"}),
            )
        }
        "browser.message_loop_status" => {
            let (beats, phase) = super::app::loop_status();
            Ok(json!({
                "beats": beats,
                "phase": phase,
                "uptimeSeconds": STARTED.with(|s| s.elapsed().as_secs()),
                "tabs": state::content_tab_ids().len(),
                "sessions": sessions::session_count(),
                "control": sessions::status()["control"],
            }))
        }
        "browser.update_status" => {
            let mut status = UPDATE.with(|u| u.borrow().clone());
            status["current"] = json!(env!("CARGO_PKG_VERSION"));
            status["automatic"] = json!(state::settings_snapshot()["autoUpdate"] != false);
            status["selfUpdating"] = json!(updates_itself());
            Ok(status)
        }
        "browser.update_check" => {
            let found = manifest(params).await?;
            let current = env!("CARGO_PKG_VERSION");
            let latest = found["version"].as_str().unwrap_or("");
            if newer(latest, current) {
                let staged = UPDATE
                    .with(|u| u.borrow()["staged"] == true && u.borrow()["available"] == latest);
                set_update_state(
                    json!({"available": latest, "staged": staged, "notes": found["notes"]}),
                );
            }
            Ok(
                json!({"current": current, "latest": latest, "available": newer(latest, current), "notes": found["notes"], "url": found["url"]}),
            )
        }
        "browser.update_stage" => {
            operator(caller, "installing an update")?;
            let found = manifest(params).await?;
            let version: String = found["version"]
                .as_str()
                .unwrap_or("")
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '.' || *c == '-')
                .collect();
            // `force` may reinstall the running version, never an older one.
            let reinstall = params["force"] == true
                && version_parts(&version) == version_parts(env!("CARGO_PKG_VERSION"));
            if !newer(&version, env!("CARGO_PKG_VERSION")) && !reinstall {
                return Ok(
                    json!({"staged": false, "reason": format!("{version} is not newer than {}", env!("CARGO_PKG_VERSION"))}),
                );
            }
            let updates = install_dir().join("updates");
            std::fs::create_dir_all(&updates).map_err(|e| Fail::internal(e.to_string()))?;
            // Already downloaded and checked (the automatic update): use it.
            let ready = updates.join(&version);
            let pending_version = std::fs::read(updates.join("pending.json"))
                .ok()
                .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                .and_then(|p| p["version"].as_str().map(str::to_string));
            if pending_version.as_deref() == Some(version.as_str())
                && ready.join("agentgo.exe").is_file()
            {
                let apply = params["apply"].as_str().unwrap_or("later");
                if apply == "now" || apply == "on_exit" {
                    schedule_swap(&ready, &updates.join("previous"), apply == "now")?;
                    if apply == "now" {
                        exec::spawn(async {
                            exec::sleep(300).await;
                            state::request_quit(0);
                        });
                    }
                }
                set_update_state(json!({"available": version, "staged": true, "applying": apply}));
                return Ok(
                    json!({"staged": true, "version": version, "path": ready.display().to_string(), "apply": apply, "reused": true}),
                );
            }
            let zip = updates.join(format!("{version}.zip"));
            fetch(found["url"].as_str().unwrap_or(""), Some(&zip)).await?;
            let bytes = std::fs::read(&zip).map_err(|e| Fail::internal(e.to_string()))?;
            let digest = agentgo_artifacts::sha256_hex(&bytes);
            if !digest.eq_ignore_ascii_case(found["sha256"].as_str().unwrap_or("")) {
                let _ = std::fs::remove_file(&zip);
                return Err(Fail::new(ErrorCode::PolicyDenied, format!("the download's SHA-256 {digest} does not match the manifest; it was deleted")));
            }
            let staged = updates.join(&version);
            let _ = std::fs::remove_dir_all(&staged);
            if let Err(reason) = super::pack::extract_zip(&bytes, &staged) {
                let _ = std::fs::remove_dir_all(&staged);
                return Err(Fail::internal(format!(
                    "could not unpack the update: {reason}"
                )));
            }
            lift_single_root(&staged);
            let pending = json!({"version": version, "staged": staged.display().to_string(), "sha256": digest});
            set_update_state(
                json!({"available": version, "staged": true, "notes": found["notes"]}),
            );
            let _ = std::fs::write(updates.join("pending.json"), pending.to_string());
            let apply = params["apply"].as_str().unwrap_or("later");
            if apply == "now" || apply == "on_exit" {
                schedule_swap(&staged, &updates.join("previous"), apply == "now")?;
                if apply == "now" {
                    exec::spawn(async {
                        exec::sleep(300).await;
                        state::request_quit(0);
                    });
                }
            }
            Ok(
                json!({"staged": true, "version": version, "path": staged.display().to_string(), "sha256": digest, "apply": apply, "next": "apply: on_exit swaps it in when AgentGo closes, now restarts into it"}),
            )
        }
        "security.status" => {
            let settings = state::settings_snapshot();
            let options = &state::state().options;
            let key = update_key();
            Ok(json!({
                "rendererSandbox": cfg!(feature = "sandbox"),
                "devtoolsPort": super::app::devtools_port_open(),
                "adBlock": settings["adBlock"],
                "adBlocker": super::adblock::status(),
                "dangerousSites": super::safesite::status(),
                "fingerprintProtection": settings["fingerprintProtection"] != false,
                // The hidden per-run symbol a protected page carries (for tests).
                "fingerprintMarker": format!("{}.fp", super::adblock::page_marker()),
                "thirdPartyCookiesBlocked": settings["blockThirdPartyCookies"] != false,
                "trackingParametersRemoved": settings["stripTracking"] != false,
                "globalPrivacyControl": true,
                "webrtc": "default_public_interface_only",
                "vpn": settings["vpn"],
                "agentFileAccess": {
                    "folders": options.upload_dirs.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
                    "note": "agents open file:// only inside these operator-granted folders",
                },
                "downloads": {"allowed": options.allow_downloads, "scanner": "Microsoft Defender", "markOfTheWeb": true},
                "updates": {"signed": true, "keyId": key.trim().chars().take(16).collect::<String>(), "downgrade": "refused"},
                "evaluateAllowed": options.allow_evaluate,
            }))
        }
        "browser.update_rollback" => {
            operator(caller, "going back to the previous version")?;
            let updates = install_dir().join("updates");
            let previous = updates.join("previous");
            if !previous.is_dir() {
                return Err(Fail::params("there is no previous version to go back to"));
            }
            // The current version becomes the "previous" of the rollback.
            let parked = updates.join("rolled-back");
            let _ = std::fs::remove_dir_all(&parked);
            std::fs::rename(&previous, &parked).map_err(|e| Fail::internal(e.to_string()))?;
            schedule_swap(&parked, &previous, params["restart"] != false)?;
            if params["restart"] != false {
                exec::spawn(async {
                    exec::sleep(300).await;
                    state::request_quit(0);
                });
            }
            Ok(json!({"rollingBack": true, "restart": params["restart"] != false}))
        }
        _ => Err(Fail::internal(format!(
            "{method} is not a lifecycle command"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_signature_is_enforced() {
        // RFC 8032 test vector 1 key; the signed text is not that vector's
        // message, so a fabricated signature must fail, as must a missing one.
        let key = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";
        let manifest = json!({"version":"9.9.9","url":"https://x/y.zip","sha256":"ab","signature":"00".repeat(64)});
        assert!(!signature_valid(&manifest, key));
        let unsigned = json!({"version":"9.9.9","url":"u","sha256":"ab"});
        assert!(!signature_valid(&unsigned, key));
        assert!(!signature_valid(&manifest, "zz"));
    }

    #[test]
    fn versions_compare() {
        assert!(newer("0.2.0", "0.1.9"));
        assert!(newer("v1.0.0", "0.9.9"));
        assert!(!newer("0.1.0", "0.1.0"));
        assert!(!newer("0.1.0-beta", "0.1.1"));
        assert!(!newer("1.2.0", "1.2"));
        assert!(newer("0.2.0", "0.2.0-rc1"));
        assert!(!newer("0.2.0-rc1", "0.2.0"));
    }
}
