//! More windows and the tab strip: `window.*` and `tab.*` beyond the basics,
//! and `frame.select/focus/navigate`.
//!
//! A secondary window holds tabs moved out of the main window (detached
//! tabs, popups, split views). It opens without taking the foreground, like
//! every AgentGo window; closing it moves its tabs back. Pinning, groups,
//! discarding and freezing are kept here and shown in the tab strip.
//!
//! Policy: agents may arrange their own windows and tabs. Moving or resizing
//! the person's main window, splitting it, and bringing any window forward
//! (which takes the foreground) are the person's: the first two need the
//! person's approval, focusing is operator only.

use std::cell::RefCell;
use std::collections::BTreeMap;

use agentgo_protocol::ErrorCode;
use cef::{ImplDisplay, ImplPanel, ImplView, ImplWindow};
use serde_json::{json, Value};

use super::sessions::{self, Caller, Fail, Reply};
use super::{cdp, exec, page, state};

struct Win {
    window: cef::Window,
    host: cef::Panel,
    owner: String,
}

#[derive(Default)]
struct Strip {
    pinned: bool,
    group: Option<(String, String)>,
    discarded: Option<(String, String)>,
    frozen: bool,
}

thread_local! {
    static WINDOWS: RefCell<BTreeMap<String, Win>> = const { RefCell::new(BTreeMap::new()) };
    static PENDING: RefCell<BTreeMap<String, exec::Slot<()>>> = const { RefCell::new(BTreeMap::new()) };
    /// tab id -> window id, for tabs outside the main window.
    static DETACHED: RefCell<BTreeMap<String, String>> = const { RefCell::new(BTreeMap::new()) };
    static STRIP: RefCell<BTreeMap<String, Strip>> = const { RefCell::new(BTreeMap::new()) };
    /// tab id -> frame id chosen with frame.select.
    static FRAMES: RefCell<BTreeMap<String, String>> = const { RefCell::new(BTreeMap::new()) };
    static PROMPTS: RefCell<Vec<Value>> = const { RefCell::new(Vec::new()) };
    static NEXT: RefCell<u64> = const { RefCell::new(0) };
}

/// The window a tab is in, when it is not the main window.
pub fn window_of(tab: &str) -> Option<String> {
    DETACHED.with(|d| d.borrow().get(tab).cloned())
}

/// The secondary host a closing tab must leave, if it is in one.
pub fn forget_tab(tab: &str) -> Option<cef::Panel> {
    STRIP.with(|s| s.borrow_mut().remove(tab));
    FRAMES.with(|f| f.borrow_mut().remove(tab));
    let id = DETACHED.with(|d| d.borrow_mut().remove(tab))?;
    let host = WINDOWS.with(|w| w.borrow().get(&id).map(|win| win.host.clone()));
    let empty = DETACHED.with(|d| !d.borrow().values().any(|w| *w == id));
    if empty {
        // The last tab left: the window goes too.
        if let Some(window) = WINDOWS.with(|w| w.borrow().get(&id).map(|win| win.window.clone())) {
            exec::spawn(async move { window.close() });
        }
    }
    host
}

/// What the tab strip shows beyond the page: window, pin, group, discarded.
pub fn strip_state(tab: &str) -> Value {
    let window = window_of(tab);
    STRIP.with(|s| {
        let all = s.borrow();
        let strip = all.get(tab);
        if window.is_none() && strip.is_none() {
            return Value::Null;
        }
        json!({
            "window": window,
            "pinned": strip.is_some_and(|s| s.pinned),
            "group": strip.and_then(|s| s.group.as_ref()).map(|(name, color)| json!({"name": name, "color": color})),
            "discarded": strip.and_then(|s| s.discarded.as_ref()).map(|(url, title)| json!({"url": url, "title": title})),
            "frozen": strip.is_some_and(|s| s.frozen),
        })
    })
}

fn strip<R>(tab: &str, f: impl FnOnce(&mut Strip) -> R) -> R {
    STRIP.with(|s| f(s.borrow_mut().entry(tab.to_string()).or_default()))
}

/// Called by the secondary window delegate once its host panel exists.
pub fn created(id: &str, window: cef::Window, host: cef::Panel) {
    let owner = PENDING
        .with(|p| p.borrow().contains_key(id))
        .then(String::new)
        .unwrap_or_default();
    WINDOWS.with(|w| {
        w.borrow_mut().insert(
            id.to_string(),
            Win {
                window,
                host,
                owner,
            },
        )
    });
    if let Some(slot) = PENDING.with(|p| p.borrow_mut().remove(id)) {
        slot.fill(());
    }
}

/// The person closes a secondary window: its tabs go back to the main one.
pub fn closing(id: &str) {
    let tabs: Vec<String> = DETACHED.with(|d| {
        d.borrow()
            .iter()
            .filter(|(_, w)| *w == id)
            .map(|(t, _)| t.clone())
            .collect()
    });
    for tab in tabs {
        let _ = attach(&tab);
    }
}

pub fn destroyed(id: &str) {
    WINDOWS.with(|w| w.borrow_mut().remove(id));
    sessions::emit("window.closed", json!({"windowId": id}));
}

fn rect(params: &Value, fallback: cef::Rect) -> cef::Rect {
    let get = |key: &str, default: i32| {
        params[key]
            .as_i64()
            .map_or(default, |v| v.clamp(-20_000, 20_000) as i32)
    };
    cef::Rect {
        x: get("x", fallback.x),
        y: get("y", fallback.y),
        width: get("width", fallback.width).max(200),
        height: get("height", fallback.height).max(150),
    }
}

fn main_bounds() -> cef::Rect {
    state::main_window()
        .map(|w| w.bounds())
        .unwrap_or(cef::Rect {
            x: 80,
            y: 80,
            width: 1100,
            height: 760,
        })
}

/// Opens a secondary window and waits until it can take tabs.
async fn open_window(caller: &Caller, bounds: cef::Rect) -> Result<String, Fail> {
    let id = NEXT.with(|n| {
        *n.borrow_mut() += 1;
        format!("window-{}", *n.borrow() + 1)
    });
    let slot = exec::Slot::default();
    PENDING.with(|p| p.borrow_mut().insert(id.clone(), slot.clone()));
    if !super::app::create_secondary_window(&id, bounds) {
        PENDING.with(|p| p.borrow_mut().remove(&id));
        return Err(Fail::internal("Chromium could not open a window"));
    }
    slot.wait(10_000)
        .await
        .ok_or_else(|| Fail::new(ErrorCode::Timeout, "the window did not open in time"))?;
    WINDOWS.with(|w| {
        if let Some(win) = w.borrow_mut().get_mut(&id) {
            win.owner = caller.session.clone();
        }
    });
    sessions::emit("window.opened", json!({"windowId": id}));
    Ok(id)
}

/// Moves a tab into a secondary window.
fn detach(tab: &str, window: &str) -> Result<(), Fail> {
    let view = state::content_view_for_tab(tab)
        .ok_or_else(|| Fail::new(ErrorCode::TargetStale, format!("no tab {tab}")))?;
    let host = WINDOWS
        .with(|w| w.borrow().get(window).map(|win| win.host.clone()))
        .ok_or_else(|| Fail::params(format!("no window {window}")))?;
    let mut as_view = cef::View::from(&view);
    match window_of(tab) {
        Some(from) => {
            if let Some(old) = WINDOWS.with(|w| w.borrow().get(&from).map(|win| win.host.clone())) {
                old.remove_child_view(Some(&mut as_view));
            }
        }
        None => {
            if state::main_window_tabs().1.len() <= 1 {
                return Err(Fail::params("the main window keeps at least one tab"));
            }
            if let Some(main) = state::tab_host() {
                main.remove_child_view(Some(&mut as_view));
            }
        }
    }
    host.add_child_view(Some(&mut as_view));
    host.reorder_child_view(Some(&mut as_view), -1);
    DETACHED.with(|d| d.borrow_mut().insert(tab.to_string(), window.to_string()));
    if let Some(win) = WINDOWS.with(|w| w.borrow().get(window).map(|win| win.window.clone())) {
        win.layout();
    }
    // The main window shows one of its own tabs.
    if state::active_tab_id() == tab {
        if let Some(next) = state::main_window_tabs().1.first() {
            let _ = state::activate_tab(next);
        }
    }
    super::app::relayout_registered_content();
    let _ = state::restack_tool_overlay();
    state::notify_shell_state();
    Ok(())
}

/// Moves a tab back into the main window.
fn attach(tab: &str) -> Result<(), Fail> {
    let from =
        window_of(tab).ok_or_else(|| Fail::params(format!("tab {tab} is in the main window")))?;
    let view = state::content_view_for_tab(tab)
        .ok_or_else(|| Fail::new(ErrorCode::TargetStale, format!("no tab {tab}")))?;
    let mut as_view = cef::View::from(&view);
    if let Some(old) = WINDOWS.with(|w| w.borrow().get(&from).map(|win| win.host.clone())) {
        old.remove_child_view(Some(&mut as_view));
    }
    DETACHED.with(|d| d.borrow_mut().remove(tab));
    if let Some(main) = state::tab_host() {
        main.add_child_view(Some(&mut as_view));
    }
    super::app::relayout_registered_content();
    let _ = state::restack_tool_overlay();
    state::notify_shell_state();
    let empty = DETACHED.with(|d| !d.borrow().values().any(|w| *w == from));
    if empty {
        if let Some(window) = WINDOWS.with(|w| w.borrow().get(&from).map(|win| win.window.clone()))
        {
            exec::spawn(async move { window.close() });
        }
    }
    Ok(())
}

/// Shows a detached tab on top inside its own window.
pub fn activate_detached(tab: &str) -> bool {
    let Some(window) = window_of(tab) else {
        return false;
    };
    if let (Some(host), Some(view)) = (
        WINDOWS.with(|w| w.borrow().get(&window).map(|win| win.host.clone())),
        state::content_view_for_tab(tab),
    ) {
        let mut as_view = cef::View::from(&view);
        host.reorder_child_view(Some(&mut as_view), -1);
    }
    true
}

fn window_param(params: &Value) -> Option<String> {
    params["windowId"]
        .as_str()
        .filter(|id| *id != "main")
        .map(str::to_string)
}

fn the_window(id: Option<&str>) -> Result<cef::Window, Fail> {
    match id {
        None => state::main_window().ok_or_else(|| Fail::internal("the main window is not ready")),
        Some(id) => WINDOWS
            .with(|w| w.borrow().get(id).map(|win| win.window.clone()))
            .ok_or_else(|| Fail::params(format!("no window {id}; window.status lists them"))),
    }
}

/// Changing the person's main window needs their approval.
fn approve_main(caller: &Caller, params: &Value, what: &str) -> Result<(), Fail> {
    let tab = page::tab_of(caller, params).unwrap_or_else(|_| state::active_tab_id());
    sessions::require_approval(
        caller,
        &tab,
        "agentgo://window",
        what,
        "change the person's window",
        params["approvalId"].as_str(),
    )
}

fn may_use_window(caller: &Caller, id: &str) -> Result<(), Fail> {
    let owner = WINDOWS.with(|w| w.borrow().get(id).map(|win| win.owner.clone()));
    match owner {
        Some(owner) if caller.is_shell() || owner == caller.session => Ok(()),
        Some(_) => Err(Fail::new(
            ErrorCode::Unauthorized,
            format!("{id} was opened by another agent"),
        )),
        None => Err(Fail::params(format!("no window {id}"))),
    }
}

fn windows_list() -> Value {
    let mut out = vec![
        json!({"windowId": "main", "bounds": bounds_of(&main_bounds()), "tabs": state::main_window_tabs().1}),
    ];
    WINDOWS.with(|w| {
        for (id, win) in w.borrow().iter() {
            let tabs: Vec<String> = DETACHED.with(|d| d.borrow().iter().filter(|(_, w)| *w == id).map(|(t, _)| t.clone()).collect());
            out.push(json!({"windowId": id, "bounds": bounds_of(&win.window.bounds()), "tabs": tabs, "owner": win.owner}));
        }
    });
    json!({"windows": out})
}

fn bounds_of(rect: &cef::Rect) -> Value {
    json!({"x": rect.x, "y": rect.y, "width": rect.width, "height": rect.height})
}

fn workspace_file() -> std::path::PathBuf {
    state::data_dir().join("workspaces.json")
}

fn chrome(call: &str, value: &Value) -> bool {
    state::chrome_script(&format!("window.{call} && window.{call}({value});"))
}

/// A message the person typed into the always visible prompt.
pub fn person_prompt(text: &str) {
    let message = json!({"text": text.chars().take(2000).collect::<String>(), "at": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())});
    PROMPTS.with(|p| {
        let mut all = p.borrow_mut();
        all.push(message.clone());
        if all.len() > 50 {
            all.remove(0);
        }
    });
    sessions::emit("person.message", message);
}

async fn frame_tree(tab: &str) -> Result<Vec<Value>, Fail> {
    let tree = cdp::call(tab, "Page.getFrameTree", json!({}))
        .await
        .map_err(Fail::internal)?;
    fn walk(node: &Value, depth: u64, out: &mut Vec<Value>) {
        let frame = &node["frame"];
        out.push(json!({"frameId": frame["id"], "name": frame["name"], "url": frame["url"], "depth": depth}));
        for child in node["childFrames"].as_array().into_iter().flatten() {
            walk(child, depth + 1, out);
        }
    }
    let mut out = Vec::new();
    walk(&tree["frameTree"], 0, &mut out);
    Ok(out)
}

/// The frame a frame.* call means: frameId, name, a url part, or the selected one.
async fn frame_of(tab: &str, params: &Value) -> Result<Value, Fail> {
    let frames = frame_tree(tab).await?;
    let selected = FRAMES.with(|f| f.borrow().get(tab).cloned());
    let found = frames.iter().find(|f| {
        if let Some(id) = params["frameId"].as_str() {
            return f["frameId"] == id;
        }
        if let Some(name) = params["name"].as_str() {
            return f["name"] == name;
        }
        if let Some(part) = params["urlContains"].as_str() {
            return f["depth"].as_u64() > Some(0)
                && f["url"].as_str().is_some_and(|u| u.contains(part));
        }
        if let Some(index) = params["index"].as_u64() {
            return f["depth"].as_u64() > Some(0)
                && frames
                    .iter()
                    .filter(|g| g["depth"].as_u64() > Some(0))
                    .position(|g| g == *f)
                    == Some(index as usize);
        }
        selected.as_deref().is_some_and(|id| f["frameId"] == id)
    });
    found.cloned().ok_or_else(|| {
        Fail::new(
            ErrorCode::TargetStale,
            "no such frame; frame.list lists them",
        )
        .with(json!({"frames": frames}))
    })
}

pub fn handles(method: &str) -> bool {
    matches!(
        method,
        "window.create"
            | "window.focus"
            | "window.activate"
            | "window.move"
            | "window.split"
            | "window.workspace"
            | "window.detach_tab"
            | "window.attach_tab"
            | "window.presentation_mode"
            | "window.agent_overlay"
            | "window.always_visible_prompt"
            | "window.list"
            | "tab.move"
            | "tab.group"
            | "tab.ungroup"
            | "tab.pin"
            | "tab.unpin"
            | "tab.discard"
            | "tab.restore"
            | "tab.open_popup"
            | "tab.sort"
            | "tab.freeze_all_background"
            | "tab.suspend_duplicates"
            | "frame.select"
            | "frame.focus"
            | "frame.navigate"
    )
}

pub async fn command(caller: &Caller, method: &str, params: &Value) -> Reply {
    match method {
        "window.list" => Ok(windows_list()),
        "window.create" | "tab.open_popup" | "window.detach_tab" => {
            let base = main_bounds();
            let fallback = if method == "tab.open_popup" {
                cef::Rect {
                    x: base.x + 60,
                    y: base.y + 60,
                    width: 520,
                    height: 640,
                }
            } else {
                cef::Rect {
                    x: base.x + 40,
                    y: base.y + 40,
                    width: base.width.min(1100),
                    height: base.height.min(800),
                }
            };
            let tab = if method == "window.detach_tab" {
                page::tab_of(caller, params)?
            } else {
                let url = params["url"].as_str().unwrap_or("about:blank");
                let created = Box::pin(super::dispatch::run(
                    caller,
                    "tabs.create",
                    json!({"url": url, "profile": params.get("profile")}),
                ))
                .await?;
                created["tabId"].as_str().unwrap_or_default().to_string()
            };
            let id = open_window(caller, rect(params, fallback)).await?;
            detach(&tab, &id)?;
            Ok(json!({"windowId": id, "tabId": tab, "url": state::tab_url(&tab)}))
        }
        "window.attach_tab" => {
            let tab = page::tab_of(caller, params)?;
            attach(&tab)?;
            Ok(json!({"tabId": tab, "windowId": "main"}))
        }
        "window.focus" | "window.activate" => {
            if !caller.is_shell() {
                return Err(Fail::new(ErrorCode::PolicyDenied, "bringing a window forward takes the person's foreground; only the person does that"));
            }
            let window = the_window(window_param(params).as_deref())?;
            super::app::bring_forward(&window);
            Ok(
                json!({"windowId": window_param(params).unwrap_or_else(|| "main".into()), "active": true}),
            )
        }
        "window.move" => {
            let id = window_param(params);
            match &id {
                Some(id) => may_use_window(caller, id)?,
                None => approve_main(caller, params, "move or resize the window")?,
            }
            let window = the_window(id.as_deref())?;
            let bounds = rect(params, window.bounds());
            window.set_bounds(Some(&bounds));
            Ok(
                json!({"windowId": id.unwrap_or_else(|| "main".into()), "bounds": bounds_of(&window.bounds())}),
            )
        }
        "window.split" => {
            let tab = page::tab_of(caller, params)?;
            approve_main(caller, params, "split the window")?;
            let main = state::main_window()
                .ok_or_else(|| Fail::internal("the main window is not ready"))?;
            let area = main
                .display()
                .map(|d| d.work_area())
                .unwrap_or_else(main_bounds);
            let ratio = params["ratio"].as_f64().unwrap_or(0.5).clamp(0.2, 0.8);
            let left = (area.width as f64 * ratio) as i32;
            if main.is_maximized() != 0 || main.is_fullscreen() != 0 {
                main.set_fullscreen(0);
                main.restore();
            }
            main.set_bounds(Some(&cef::Rect {
                x: area.x,
                y: area.y,
                width: left,
                height: area.height,
            }));
            let right = cef::Rect {
                x: area.x + left,
                y: area.y,
                width: area.width - left,
                height: area.height,
            };
            let id = match window_of(&tab) {
                Some(id) => id,
                None => {
                    let id = open_window(caller, right.clone()).await?;
                    detach(&tab, &id)?;
                    id
                }
            };
            the_window(Some(&id))?.set_bounds(Some(&right));
            Ok(json!({"left": "main", "right": id, "tabId": tab}))
        }
        "window.workspace" => {
            let mut all: BTreeMap<String, Value> = std::fs::read(workspace_file())
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
                .unwrap_or_default();
            let name = params["name"]
                .as_str()
                .unwrap_or("default")
                .chars()
                .take(64)
                .collect::<String>();
            match params["action"].as_str().unwrap_or("list") {
                "list" => Ok(json!({"workspaces": all.keys().collect::<Vec<_>>()})),
                "save" => {
                    let layout: Vec<Value> = windows_list()["windows"]
                        .as_array()
                        .cloned()
                        .unwrap_or_default()
                        .into_iter()
                        .map(|w| {
                            let urls: Vec<String> = w["tabs"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .filter_map(|t| t.as_str())
                                .map(state::tab_url)
                                .filter(|u| state::is_navigable_url(u))
                                .collect();
                            json!({"windowId": w["windowId"], "bounds": w["bounds"], "urls": urls})
                        })
                        .collect();
                    all.insert(name.clone(), json!(layout));
                    let _ = std::fs::write(
                        workspace_file(),
                        serde_json::to_vec_pretty(&all).unwrap_or_default(),
                    );
                    Ok(json!({"saved": name, "windows": layout.len()}))
                }
                "delete" => Ok(
                    json!({"deleted": all.remove(&name).is_some(), "written": std::fs::write(workspace_file(), serde_json::to_vec_pretty(&all).unwrap_or_default()).is_ok()}),
                ),
                "restore" => {
                    let layout = all
                        .get(&name)
                        .cloned()
                        .ok_or_else(|| Fail::params(format!("no workspace {name}")))?;
                    let mut opened = Vec::new();
                    for window in layout.as_array().cloned().unwrap_or_default() {
                        let urls: Vec<String> = window["urls"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|u| u.as_str().map(str::to_string))
                            .collect();
                        if window["windowId"] == "main" {
                            for url in urls {
                                let tab = Box::pin(super::dispatch::run(
                                    caller,
                                    "tabs.create",
                                    json!({"url": url}),
                                ))
                                .await?;
                                opened.push(tab["tabId"].clone());
                            }
                            continue;
                        }
                        let Some((first, rest)) = urls.split_first() else {
                            continue;
                        };
                        let made = Box::pin(command(caller, "window.create", &json!({"url": first, "x": window["bounds"]["x"], "y": window["bounds"]["y"], "width": window["bounds"]["width"], "height": window["bounds"]["height"]}))).await?;
                        opened.push(made["tabId"].clone());
                        for url in rest {
                            let tab = Box::pin(super::dispatch::run(
                                caller,
                                "tabs.create",
                                json!({"url": url}),
                            ))
                            .await?;
                            let tab_id = tab["tabId"].as_str().unwrap_or_default().to_string();
                            detach(&tab_id, made["windowId"].as_str().unwrap_or_default())?;
                            opened.push(json!(tab_id));
                        }
                    }
                    Ok(json!({"restored": name, "tabs": opened}))
                }
                _ => Err(Fail::params("action: list, save, restore or delete")),
            }
        }
        "window.presentation_mode" => {
            let on = params["on"] != false;
            let window = state::main_window()
                .ok_or_else(|| Fail::internal("the main window is not ready"))?;
            if let Some(shell) = state::shell_view() {
                shell.set_visible(i32::from(!on));
            }
            window.set_fullscreen(i32::from(on));
            window.layout();
            super::app::relayout_registered_content();
            Ok(
                json!({"presentation": on, "note": if on { "only the page shows; window.presentation_mode {on:false} or Esc in the page's own full screen brings the bar back" } else { "" }}),
            )
        }
        "window.agent_overlay" => {
            let on = params["on"] != false;
            let text: String = params["text"]
                .as_str()
                .unwrap_or("Agent at work")
                .chars()
                .take(120)
                .collect();
            let shown = chrome(
                "__agentgoOverlay",
                &json!({"on": on, "text": text, "session": caller.session}),
            );
            Ok(json!({"overlay": on, "text": text, "shown": shown}))
        }
        "window.always_visible_prompt" => {
            if params["read"] == true {
                let messages = PROMPTS.with(|p| std::mem::take(&mut *p.borrow_mut()));
                return Ok(json!({"messages": messages}));
            }
            let on = params["on"] != false;
            let placeholder: String = params["placeholder"]
                .as_str()
                .unwrap_or("Tell the agent something")
                .chars()
                .take(80)
                .collect();
            let shown = chrome(
                "__agentgoPrompt",
                &json!({"on": on, "placeholder": placeholder}),
            );
            Ok(
                json!({"prompt": on, "shown": shown, "read": "window.always_visible_prompt {read:true} returns what the person typed; the event person.message arrives too"}),
            )
        }
        "tab.move" => {
            let tab = page::tab_of(caller, params)?;
            let mut order = state::content_tab_ids();
            order.retain(|t| *t != tab);
            let index = (params["index"].as_u64().unwrap_or(0) as usize).min(order.len());
            order.insert(index, tab.clone());
            state::reorder_tabs(&order);
            Ok(json!({"tabId": tab, "index": index, "order": order}))
        }
        "tab.pin" | "tab.unpin" => {
            let tab = page::tab_of(caller, params)?;
            let on = method == "tab.pin";
            strip(&tab, |s| s.pinned = on);
            // Pinned tabs lead the strip.
            let mut order = state::content_tab_ids();
            order.sort_by_key(|t| !STRIP.with(|s| s.borrow().get(t).is_some_and(|s| s.pinned)));
            state::reorder_tabs(&order);
            Ok(json!({"tabId": tab, "pinned": on}))
        }
        "tab.group" | "tab.ungroup" => {
            let ids: Vec<String> = match params["tabIds"].as_array() {
                Some(list) => list
                    .iter()
                    .filter_map(|t| t.as_str().map(str::to_string))
                    .collect(),
                None => vec![page::tab_of(caller, params)?],
            };
            for tab in &ids {
                sessions::authorize_tab(caller, tab)?;
            }
            let group = (method == "tab.group").then(|| {
                let name: String = params["name"]
                    .as_str()
                    .unwrap_or("Group")
                    .chars()
                    .take(40)
                    .collect();
                let color = params["color"]
                    .as_str()
                    .filter(|c| {
                        c.len() <= 20 && c.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '#')
                    })
                    .unwrap_or("#5B8CFF")
                    .to_string();
                (name, color)
            });
            for tab in &ids {
                strip(tab, |s| s.group = group.clone());
            }
            // A group sits together, where its first tab is.
            if let Some((name, _)) = &group {
                let all = state::content_tab_ids();
                let first = all.iter().position(|t| ids.contains(t)).unwrap_or(0);
                let mut order: Vec<String> =
                    all.iter().filter(|t| !ids.contains(t)).cloned().collect();
                let members: Vec<String> =
                    all.iter().filter(|t| ids.contains(t)).cloned().collect();
                let at = first.min(order.len());
                order.splice(at..at, members);
                state::reorder_tabs(&order);
                return Ok(json!({"group": name, "tabs": ids}));
            }
            state::notify_shell_state();
            Ok(json!({"ungrouped": ids}))
        }
        "tab.sort" => {
            let by = params["by"].as_str().unwrap_or("domain");
            let mut tabs: Vec<(String, String, String)> = state::content_tab_ids()
                .into_iter()
                .map(|t| {
                    let url = state::tab_url(&t);
                    let title = state::tabs_snapshot()
                        .into_iter()
                        .find(|s| s["tabId"] == t.as_str())
                        .and_then(|s| s["title"].as_str().map(str::to_string))
                        .unwrap_or_default();
                    (t, url, title)
                })
                .collect();
            let host = |url: &str| {
                url.split("://")
                    .nth(1)
                    .unwrap_or(url)
                    .split('/')
                    .next()
                    .unwrap_or("")
                    .trim_start_matches("www.")
                    .to_string()
            };
            let pinned = |t: &str| !STRIP.with(|s| s.borrow().get(t).is_some_and(|s| s.pinned));
            match by {
                "title" => tabs.sort_by_key(|(t, _, title)| (pinned(t), title.to_lowercase())),
                "url" => tabs.sort_by_key(|(t, url, _)| (pinned(t), url.clone())),
                "domain" => tabs.sort_by_key(|(t, url, _)| (pinned(t), host(url))),
                _ => return Err(Fail::params("by: domain, title or url")),
            }
            let order: Vec<String> = tabs.into_iter().map(|(t, _, _)| t).collect();
            state::reorder_tabs(&order);
            Ok(json!({"by": by, "order": order}))
        }
        "tab.discard" => {
            let tab = page::tab_of(caller, params)?;
            if state::active_tab_id() == tab && window_of(&tab).is_none() && caller.is_shell() {
                return Err(Fail::params(
                    "the tab the person is looking at is not discarded",
                ));
            }
            let url = state::tab_url(&tab);
            let title = state::tabs_snapshot()
                .into_iter()
                .find(|s| s["tabId"] == tab.as_str())
                .and_then(|s| s["title"].as_str().map(str::to_string))
                .unwrap_or_default();
            if url == "about:blank" {
                return Ok(json!({"tabId": tab, "discarded": false, "reason": "nothing to free"}));
            }
            strip(&tab, |s| s.discarded = Some((url.clone(), title)));
            state::navigate_tab(&tab, "about:blank");
            Ok(json!({"tabId": tab, "discarded": true, "url": url}))
        }
        "tab.restore" => {
            let tab = page::tab_of(caller, params)?;
            Ok(restore(&tab).map_or_else(
                || json!({"tabId": tab, "restored": false}),
                |url| json!({"tabId": tab, "restored": true, "url": url}),
            ))
        }
        "tab.freeze_all_background" => {
            let active = state::active_tab_id();
            let mut frozen = Vec::new();
            for tab in state::content_tab_ids() {
                let busy =
                    sessions::lease_holder(&tab).is_some_and(|holder| holder != caller.session);
                if tab == active
                    || busy
                    || window_of(&tab).is_some()
                    || sessions::default_tab(caller).as_deref() == Some(tab.as_str())
                {
                    continue;
                }
                if cdp::call(
                    &tab,
                    "Page.setWebLifecycleState",
                    json!({"state": "frozen"}),
                )
                .await
                .is_ok()
                {
                    strip(&tab, |s| s.frozen = true);
                    frozen.push(tab);
                }
            }
            state::notify_shell_state();
            Ok(
                json!({"frozen": frozen, "note": "a frozen tab runs again when it is shown or with tab.unfreeze"}),
            )
        }
        "tab.suspend_duplicates" => {
            let mut seen: BTreeMap<String, String> = BTreeMap::new();
            let mut suspended = Vec::new();
            let active = state::active_tab_id();
            let mut ids = state::content_tab_ids();
            // The visible tab is the one that stays.
            ids.sort_by_key(|t| *t != active);
            for tab in ids {
                let url = state::tab_url(&tab);
                if !url.starts_with("http") || !sessions::can_see_tab(caller, &tab) {
                    continue;
                }
                match seen.get(&url).cloned() {
                    Some(first) => {
                        if let Ok(out) =
                            Box::pin(command(caller, "tab.discard", &json!({"tabId": tab}))).await
                        {
                            if out["discarded"] == true {
                                suspended.push(json!({"tabId": tab, "sameAs": first, "url": url}));
                            }
                        }
                    }
                    None => {
                        seen.insert(url, tab);
                    }
                }
            }
            Ok(json!({"suspended": suspended}))
        }
        "frame.select" => {
            let tab = page::tab_of(caller, params)?;
            if params["top"] == true {
                FRAMES.with(|f| f.borrow_mut().remove(&tab));
                return Ok(json!({"selected": "top"}));
            }
            let frame = frame_of(&tab, params).await?;
            FRAMES.with(|f| {
                f.borrow_mut().insert(
                    tab.clone(),
                    frame["frameId"].as_str().unwrap_or_default().to_string(),
                )
            });
            Ok(json!({"selected": frame}))
        }
        "frame.focus" => {
            let tab = page::tab_of(caller, params)?;
            let frame = frame_of(&tab, params).await?;
            let owner = cdp::call(
                &tab,
                "DOM.getFrameOwner",
                json!({"frameId": frame["frameId"]}),
            )
            .await
            .map_err(Fail::internal)?;
            cdp::call(
                &tab,
                "DOM.focus",
                json!({"backendNodeId": owner["backendNodeId"]}),
            )
            .await
            .map_err(Fail::internal)?;
            Ok(json!({"focused": frame}))
        }
        "frame.navigate" => {
            let tab = page::tab_of(caller, params)?;
            let url = params["url"]
                .as_str()
                .ok_or_else(|| Fail::params("url is required"))?;
            if !state::is_navigable_url(url) {
                return Err(Fail::params("url must use http, https, file or about"));
            }
            if let Some(host) = super::dispatch::web_origin(url)
                .and_then(|o| o.split("://").nth(1).map(str::to_string))
            {
                super::suite::domain_allowed(&host)?;
            }
            let frame = frame_of(&tab, params).await?;
            if frame["depth"] == 0 {
                return Err(Fail::params("that is the top frame; use navigate.url"));
            }
            let out = cdp::call(
                &tab,
                "Page.navigate",
                json!({"url": url, "frameId": frame["frameId"]}),
            )
            .await
            .map_err(Fail::internal)?;
            if let Some(error) = out["errorText"].as_str().filter(|e| !e.is_empty()) {
                return Err(Fail::internal(format!("the frame did not load: {error}")));
            }
            Ok(json!({"frameId": frame["frameId"], "url": url}))
        }
        _ => Err(Fail::internal(format!("{method} is not a window command"))),
    }
}

/// Reloads a discarded tab; called when it is shown again.
pub fn restore(tab: &str) -> Option<String> {
    let (url, _) = STRIP.with(|s| s.borrow_mut().get_mut(tab).and_then(|s| s.discarded.take()))?;
    state::navigate_tab(tab, &url);
    state::notify_shell_state();
    Some(url)
}

/// A tab is shown: it runs again if it was frozen, and reloads if discarded.
pub async fn shown(tab: &str) {
    if STRIP
        .with(|s| {
            s.borrow_mut()
                .get_mut(tab)
                .map(|s| std::mem::take(&mut s.frozen))
        })
        .unwrap_or(false)
    {
        let _ = cdp::call(tab, "Page.setWebLifecycleState", json!({"state": "active"})).await;
    }
    restore(tab);
}
