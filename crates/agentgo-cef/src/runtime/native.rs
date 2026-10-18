//! The native Windows lane: other programs' windows through UI Automation
//! (`native.ps1`, run per call off the UI thread).
//!
//! Policy: an agent may touch other programs only after the operator turned
//! on `nativeControl`, and every action (click, type, keys, moving windows,
//! the system clipboard, dialogs) also needs the person's approval of exactly
//! that action. Elements are clicked through UI Automation patterns or
//! window messages, so the person's focus and cursor stay where they are;
//! only `hotkey`, `drag_drop` and `double_click {real:true}` use real input,
//! bring the window forward for a moment and give focus back.

use serde_json::{json, Value};

use super::sessions::{self, Caller, Fail, Reply};
use super::{exec, page, state};
use agentgo_protocol::ErrorCode;

const SCRIPT: &str = include_str!("native.ps1");

fn script_path() -> Result<std::path::PathBuf, Fail> {
    // One copy per build, so an older browser never runs a newer script.
    let hash = SCRIPT.bytes().fold(0xcbf29ce484222325u64, |h, b| {
        (h ^ b as u64).wrapping_mul(0x100000001b3)
    });
    let path = std::env::temp_dir().join(format!("agentgo-native-{hash:016x}.ps1"));
    // UTF-8 with BOM: Windows PowerShell reads BOM-less files as ANSI.
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice(SCRIPT.as_bytes());
    // Checked on every use: a copy that was cut short or changed by another
    // program is replaced before it runs.
    if std::fs::read(&path).ok().as_deref() != Some(bytes.as_slice()) {
        std::fs::write(&path, bytes).map_err(|error| Fail::internal(error.to_string()))?;
    }
    Ok(path)
}

/// Runs one helper operation and maps its `{error}` reply to a typed failure.
pub async fn helper(request: Value, timeout_ms: u64) -> Reply {
    let path = script_path()?;
    let input = request.to_string();
    let output = exec::blocking(timeout_ms, move || {
        use std::io::Write;
        #[cfg(windows)]
        use std::os::windows::process::CommandExt;
        let mut command = std::process::Command::new("powershell");
        command
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ])
            .arg(&path)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        #[cfg(windows)]
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        let mut child = command.spawn().map_err(|error| error.to_string())?;
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(input.as_bytes());
        }
        child.wait_with_output().map_err(|error| error.to_string())
    })
    .await
    .ok_or_else(|| {
        Fail::new(
            ErrorCode::Timeout,
            "the native helper did not answer in time",
        )
    })?
    .map_err(Fail::internal)?;
    let text = String::from_utf8_lossy(&output.stdout);
    let value: Value = serde_json::from_str(text.trim()).map_err(|_| {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = if stderr.trim().is_empty() {
            text.chars().take(300).collect::<String>()
        } else {
            stderr.lines().take(3).collect::<Vec<_>>().join(" ")
        };
        Fail::internal(format!(
            "the native helper failed (exit {:?}): {detail}",
            output.status.code()
        ))
    })?;
    if let Some(error) = value.get("error") {
        let code = error["code"]
            .as_str()
            .and_then(ErrorCode::from_wire)
            .unwrap_or(ErrorCode::Internal);
        return Err(Fail::new(
            code,
            error["message"]
                .as_str()
                .unwrap_or("native helper error")
                .to_string(),
        ));
    }
    Ok(value)
}

pub fn own_hwnd() -> i64 {
    use cef::ImplWindow;
    state::state()
        .window
        .lock()
        .ok()
        .and_then(|slot| {
            slot.as_ref()
                .map(|handle| handle.0.window_handle().0 as i64)
        })
        .unwrap_or(0)
}

/// What an action names, for the approval prompt.
fn describe(params: &Value) -> String {
    let window = params["window"]
        .as_str()
        .map(str::to_string)
        .or_else(|| params["hwnd"].as_i64().map(|h| format!("window {h}")));
    let element = ["name", "automationId", "path", "option", "key", "side"]
        .iter()
        .find_map(|key| {
            params
                .get(*key)
                .map(|v| format!("{key} {}", v.as_str().map_or(v.to_string(), str::to_string)))
        });
    [window, element]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(", ")
}

/// `native.*` commands. `browser.window_native` style questions about
/// AgentGo's own window (`window.native_handle`) need no permission.
pub async fn command(caller: &Caller, method: &str, params: &Value) -> Reply {
    let name = method.trim_start_matches("native.");
    // A notice an agent raises says it is the agent's, so it cannot pass for
    // Windows, a bank or the person's other programs.
    let from_agent = |text: &str| -> String {
        if caller.is_shell() {
            text.to_string()
        } else {
            let client = sessions::client_of(&caller.session);
            if client == "agent" {
                format!("Agent: {text}")
            } else {
                format!(
                    "Agent {}: {text}",
                    client.chars().take(40).collect::<String>()
                )
            }
        }
    };
    // Commands about AgentGo itself or the person's own settings.
    match name {
        "accessibility" => return helper(json!({"op": "accessibility"}), 20_000).await,
        "multi_monitor" => return helper(json!({"op": "monitors"}), 20_000).await,
        "taskbar" | "notification" => {
            let message = params["text"]
                .as_str()
                .unwrap_or("AgentGo needs your attention");
            if params["flash"] != false || name == "notification" {
                state::notify_operator(&from_agent(&message.chars().take(200).collect::<String>()));
            }
            return Ok(
                json!({"window": own_hwnd(), "flashed": true, "notified": name == "notification"}),
            );
        }
        "toast" => {
            let title = from_agent(
                &params["title"]
                    .as_str()
                    .unwrap_or("AgentGo")
                    .chars()
                    .take(80)
                    .collect::<String>(),
            );
            let text: String = params["text"]
                .as_str()
                .unwrap_or("")
                .chars()
                .take(300)
                .collect();
            return helper(json!({"op": "toast", "title": title, "text": text}), 20_000).await;
        }
        "global_hotkey" => return hotkeys::command(caller, params),
        _ => {}
    }
    if !caller.is_shell() && state::settings_snapshot()["nativeControl"] != true {
        return Err(Fail::new(
            ErrorCode::PolicyDenied,
            "control of other programs is off; the person can turn on \"Let agents use other programs\" (nativeControl) in AgentGo settings",
        ));
    }
    let mut request = params.clone();
    if let Some(map) = request.as_object_mut() {
        map.remove("approvalId");
        map.remove("tabId");
    }
    let (op, acts) = match name {
        "window_list" => ("windows", false),
        "ui_tree" => ("tree", false),
        "find" => ("find", false),
        "read_value" => ("read", false),
        "window_bounds" => (
            "bounds",
            ["x", "y", "width", "height"]
                .iter()
                .any(|k| params.get(*k).is_some()),
        ),
        "window_snap" => ("snap", true),
        "window_focus" => ("focus", true),
        "click" => ("click", true),
        "double_click" => ("double_click", true),
        "type" => ("type", true),
        "key" => ("key", true),
        "hotkey" => ("hotkey", true),
        "select" => ("select", true),
        "expand" => ("expand", true),
        "collapse" => ("collapse", true),
        "scroll" => ("scroll", true),
        "drag_drop" => ("drag", true),
        // The system clipboard holds the person's data, so reading it is
        // approved like writing it.
        "clipboard" => ("clipboard", true),
        "file_dialog" | "save_dialog" | "print_dialog" | "permission_dialog" => {
            let acting = params.get("button").is_some() || params.get("fileName").is_some();
            (if acting { "dialog" } else { "dialogs" }, acting)
        }
        "system_tray" => ("tray", params.get("name").is_some()),
        _ => {
            return Err(Fail::new(
                ErrorCode::CapabilityUnsupported,
                format!("{method} is not a native command"),
            ))
        }
    };
    if acts {
        let tab = page::tab_of(caller, params).unwrap_or_else(|_| state::active_tab_id());
        sessions::require_approval(
            caller,
            &tab,
            "agentgo://native",
            &format!("{name}: {}", describe(params)),
            "act in another program",
            params["approvalId"].as_str(),
        )?;
    }
    request["op"] = json!(op);
    let mut out = helper(request, 60_000).await?;
    if name == "window_list" {
        // Mark AgentGo's own window so an agent does not drive itself.
        let own = own_hwnd();
        if let Some(list) = out["windows"].as_array_mut() {
            for window in list {
                if window["hwnd"].as_i64() == Some(own) {
                    window["agentgo"] = json!(true);
                }
            }
        }
    }
    Ok(out)
}

/// System-wide shortcuts that run an AgentGo command for the person, for
/// example Ctrl+Alt+P to pause every agent. Operator only.
pub mod hotkeys {
    use super::*;
    use std::sync::Mutex;

    /// (id, modifiers, key, command) registered on the hotkey thread.
    static WANTED: Mutex<Vec<(i32, u32, u32, String)>> = Mutex::new(Vec::new());
    static THREAD: Mutex<Option<u32>> = Mutex::new(None);
    const ALLOWED: [&str; 6] = [
        "agent.pause",
        "agent.resume",
        "agent.stop",
        "window.restore",
        "browser_ui.new_tab",
        "shell.command_bar",
    ];

    fn parse(combo: &str) -> Option<(u32, u32)> {
        let mut modifiers = 0x4000; // MOD_NOREPEAT
        let mut key = None;
        for part in combo.split('+').map(|p| p.trim().to_ascii_lowercase()) {
            match part.as_str() {
                "ctrl" | "control" => modifiers |= 0x2,
                "alt" => modifiers |= 0x1,
                "shift" => modifiers |= 0x4,
                "win" => modifiers |= 0x8,
                p if p.len() == 1 && p.chars().all(|c| c.is_ascii_alphanumeric()) => {
                    key = Some(p.to_ascii_uppercase().as_bytes()[0] as u32)
                }
                p if p.starts_with('f')
                    && p[1..].parse::<u32>().is_ok_and(|n| (1..=12).contains(&n)) =>
                {
                    key = Some(0x6F + p[1..].parse::<u32>().ok()?)
                }
                _ => return None,
            }
        }
        (modifiers & 0xF != 0).then_some(())?;
        key.map(|key| (modifiers, key))
    }

    pub fn command(caller: &Caller, params: &Value) -> Reply {
        let list = || {
            WANTED
                .lock()
                .map(|w| {
                    w.iter()
                        .map(|(id, _, _, cmd)| json!({"id": id, "command": cmd}))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        };
        let Some(combo) = params["keys"].as_str() else {
            return Ok(json!({"hotkeys": list(), "commands": ALLOWED}));
        };
        if !caller.is_shell() {
            return Err(Fail::new(
                ErrorCode::PolicyDenied,
                "system-wide shortcuts are set by the person in AgentGo",
            ));
        }
        let (modifiers, key) = parse(combo).ok_or_else(|| {
            Fail::params("keys: a combination such as Ctrl+Alt+P (needs Ctrl, Alt, Shift or Win)")
        })?;
        let id = 0x5000 + (modifiers as i32 & 0xF) * 256 + key as i32;
        let command = params["command"].as_str().unwrap_or("");
        if params["remove"] == true {
            if let Ok(mut wanted) = WANTED.lock() {
                wanted.retain(|entry| entry.0 != id);
            }
        } else {
            if !ALLOWED.contains(&command) {
                return Err(Fail::params(format!("command must be one of {ALLOWED:?}")));
            }
            if let Ok(mut wanted) = WANTED.lock() {
                wanted.retain(|entry| entry.0 != id);
                wanted.push((id, modifiers, key, command.to_string()));
            }
        }
        let registered = restart()?;
        Ok(json!({"keys": combo, "registered": registered, "hotkeys": list()}))
    }

    /// (Re)starts the thread that owns the registrations; returns whether
    /// every shortcut could be registered (another program may own one).
    #[cfg(windows)]
    fn restart() -> Result<bool, Fail> {
        use windows_sys::Win32::System::Threading::GetCurrentThreadId;
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, UnregisterHotKey};
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GetMessageW, PostThreadMessageW, MSG, WM_HOTKEY, WM_QUIT,
        };
        if let Some(thread) = THREAD.lock().ok().and_then(|mut t| t.take()) {
            // SAFETY: posting a message to a thread id has no memory effects.
            unsafe { PostThreadMessageW(thread, WM_QUIT, 0, 0) };
        }
        let wanted = WANTED.lock().map(|w| w.clone()).unwrap_or_default();
        if wanted.is_empty() {
            return Ok(true);
        }
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            // SAFETY: registrations belong to this thread and are removed
            // before it ends; MSG is plain data filled by GetMessageW.
            unsafe {
                let ok = wanted
                    .iter()
                    .all(|(id, m, k, _)| RegisterHotKey(std::ptr::null_mut(), *id, *m, *k) != 0);
                let _ = tx.send((GetCurrentThreadId(), ok));
                let mut msg: MSG = std::mem::zeroed();
                while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                    if msg.message == WM_HOTKEY {
                        if let Some((_, _, _, command)) =
                            wanted.iter().find(|entry| entry.0 as usize == msg.wParam)
                        {
                            let command = command.clone();
                            exec::post(move || {
                                exec::spawn(async move {
                                    let _ = super::super::dispatch::dispatch(
                                        &Caller::shell(),
                                        &command,
                                        json!({}),
                                    )
                                    .await;
                                });
                            });
                            state::wake_ui();
                        }
                    }
                }
                for (id, ..) in &wanted {
                    UnregisterHotKey(std::ptr::null_mut(), *id);
                }
            }
        });
        let (thread, ok) = rx
            .recv_timeout(std::time::Duration::from_secs(3))
            .map_err(|_| Fail::internal("hotkey thread did not start"))?;
        if let Ok(mut slot) = THREAD.lock() {
            *slot = Some(thread);
        }
        Ok(ok)
    }

    #[cfg(not(windows))]
    fn restart() -> Result<bool, Fail> {
        Ok(false)
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn parses_combinations() {
            assert_eq!(super::parse("Ctrl+Alt+P"), Some((0x4003, b'P' as u32)));
            assert_eq!(super::parse("Shift+F5"), Some((0x4004, 0x74)));
            assert_eq!(super::parse("P"), None);
            assert_eq!(super::parse("Ctrl+Nope"), None);
        }
    }
}
