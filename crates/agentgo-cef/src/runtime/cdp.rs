//! In-process DevTools protocol client for content tabs.
//!
//! CEF exposes the Chromium DevTools protocol to the browser process without
//! opening a debugging port. AgentGo uses it for observation, trusted input,
//! screenshots, PDF, cookies, and network metadata. Everything here runs on
//! the CEF UI thread.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};

use cef::*;
use serde_json::{json, Value};

use super::exec::Slot;

/// Name of the isolated JavaScript world that hosts the AgentGo page agent.
/// Page scripts cannot see or modify anything defined in this world.
const WORLD: &str = "agentgo";
/// Page-side observation/action helper, installed in the isolated world.
const AGENT_JS: &str = concat!(include_str!("agent.js"), "\n", include_str!("tools.js"));
const EVENT_RING: usize = 400;

type CdpResult = Result<Value, String>;

#[derive(Default)]
struct TabCdp {
    browser_id: i32,
    _registration: Option<Registration>,
    /// Whether the Page/Runtime domains are enabled on the current DevTools
    /// session. Chromium drops the session on cross-process navigations.
    enabled: bool,
    /// frameId -> isolated-world execution context id.
    contexts: HashMap<String, i64>,
    main_frame: Option<String>,
    /// Bumped on every main-frame navigation.
    generation: u64,
    /// Open JavaScript dialog awaiting a decision.
    dialog: Option<Value>,
    /// Data of a drag the page started, held for `act.drag`.
    drag: Option<Value>,
    /// Bounded network/console metadata ring (never bodies).
    events: VecDeque<Value>,
    /// Lines of the last actionable snapshot, for delta computation.
    pub last_lines: Vec<String>,
    /// Highest element reference handed out, so refs never repeat across
    /// documents in one tab.
    pub ref_base: u64,
}

#[derive(Default)]
struct State {
    next_id: i32,
    pending: HashMap<i32, Slot<CdpResult>>,
    tabs: HashMap<String, TabCdp>,
}

thread_local! {
    static CDP: RefCell<State> = RefCell::new(State::default());
}

fn tab_for_browser(state: &mut State, browser_id: i32) -> Option<(&String, &mut TabCdp)> {
    state
        .tabs
        .iter_mut()
        .find(|(_, tab)| tab.browser_id == browser_id)
}

fn parse(bytes: Option<&[u8]>) -> Value {
    bytes
        .and_then(|bytes| serde_json::from_slice(bytes).ok())
        .unwrap_or(Value::Null)
}

wrap_dev_tools_message_observer! {
    struct Observer;

    impl DevToolsMessageObserver {
        fn on_dev_tools_method_result(
            &self,
            _browser: Option<&mut Browser>,
            message_id: i32,
            success: i32,
            result: Option<&[u8]>,
        ) {
            if std::env::var_os("AGENTGO_CDP_TRACE").is_some() { eprintln!("CDP-RESULT {message_id} {success} {}", String::from_utf8_lossy(result.unwrap_or_default()).chars().take(160).collect::<String>()); }
            let value = parse(result);
            let slot = CDP.with(|cdp| cdp.borrow_mut().pending.remove(&message_id));
            if let Some(slot) = slot {
                slot.fill(if success != 0 {
                    Ok(value)
                } else {
                    Err(value
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("DevTools method failed")
                        .to_string())
                });
            }
        }

        fn on_dev_tools_event(
            &self,
            browser: Option<&mut Browser>,
            method: Option<&CefString>,
            params: Option<&[u8]>,
        ) {
            let (Some(browser), Some(method)) = (browser, method) else {
                return;
            };
            let method = method.to_string();
            let params = parse(params);
            let browser_id = browser.identifier();
            if std::env::var_os("AGENTGO_CDP_TRACE").is_some() { eprintln!("CDP-EVENT {browser_id} {method}"); }
            if method == "Fetch.authRequired" {
                let tab = CDP.with(|cdp| {
                    tab_for_browser(&mut cdp.borrow_mut(), browser_id).map(|(id, _)| id.clone())
                });
                if let Some(tab) = tab {
                    super::exec::spawn(super::dialogs::fetch_auth(tab, params.clone()));
                }
                return;
            }
            if method == "Fetch.requestPaused" {
                let tab = CDP.with(|cdp| {
                    tab_for_browser(&mut cdp.borrow_mut(), browser_id).map(|(id, _)| id.clone())
                });
                if let Some(tab) = tab {
                    super::exec::spawn(super::netctl::on_paused(tab, params.clone()));
                }
                return;
            }
            if method == "Page.loadEventFired" {
                let tab = CDP.with(|cdp| {
                    tab_for_browser(&mut cdp.borrow_mut(), browser_id).map(|(id, _)| id.clone())
                });
                if let Some(tab) = tab {
                    super::exec::spawn(super::extensions::inject(tab.clone()));
                    super::exec::spawn(super::misc::inject_user_scripts(tab));
                }
            }
            if method == "Runtime.executionContextCreated"
                && params["context"]["auxData"]["isDefault"] == true
            {
                // A new document's own world: make sure it is protected even
                // when it started before the protection script was registered.
                let tab = CDP.with(|cdp| {
                    tab_for_browser(&mut cdp.borrow_mut(), browser_id).map(|(id, _)| id.clone())
                });
                if let (Some(tab), Some(context)) = (tab, params["context"]["id"].as_i64()) {
                    super::exec::spawn(super::fingerprint::ensure(tab, context));
                }
            }
            if method == "Runtime.bindingCalled" {
                let tab = CDP.with(|cdp| {
                    tab_for_browser(&mut cdp.borrow_mut(), browser_id).map(|(id, _)| id.clone())
                });
                if let (true, Some(tab), Some(payload)) =
                    (params["name"] == "__agReport", tab.clone(), params["payload"].as_str())
                {
                    super::accounts::page_report(&tab, payload);
                    return;
                }
                if let (true, Some(tab), Some(payload)) =
                    (params["name"] == "__agExtTabs", tab.as_deref(), params["payload"].as_str())
                {
                    super::extensions::tabs_request(tab, payload);
                    return;
                }
                if let (true, Some(payload)) = (params["name"] == "__agExt", params["payload"].as_str()) {
                    if let (Some(tab), Some(context)) = (tab.as_deref(), params["executionContextId"].as_i64()) {
                        super::extensions::storage_from_content(tab, context, payload);
                    }
                    return;
                }
                // A binding an agent added (runtime.add_binding).
                if let Some(tab) = tab {
                    CDP.with(|cdp| {
                        if let Some(tab) = cdp.borrow_mut().tabs.get_mut(&tab) {
                            push_event(tab, json!({
                                "kind": "binding",
                                "name": params["name"],
                                "payload": params["payload"].as_str().map(|p| p.chars().take(4000).collect::<String>()),
                            }));
                        }
                    });
                }
                return;
            }
            CDP.with(|cdp| {
                let mut state = cdp.borrow_mut();
                let Some((_, tab)) = tab_for_browser(&mut state, browser_id) else {
                    return;
                };
                match method.as_str() {
                    "Runtime.executionContextCreated" => {
                        let context = &params["context"];
                        if context["name"] == WORLD {
                            if let (Some(id), Some(frame)) = (
                                context["id"].as_i64(),
                                context["auxData"]["frameId"].as_str(),
                            ) {
                                tab.contexts.insert(frame.to_string(), id);
                            }
                        }
                    }
                    "Runtime.executionContextDestroyed" => {
                        let id = params["executionContextId"].as_i64();
                        tab.contexts.retain(|_, context| Some(*context) != id);
                    }
                    "Runtime.executionContextsCleared" => tab.contexts.clear(),
                    "Page.frameNavigated" if params["frame"]["parentId"].is_null() => {
                        tab.main_frame = params["frame"]["id"].as_str().map(str::to_string);
                    }
                    "Inspector.detached" => {
                        tab.enabled = false;
                        tab.contexts.clear();
                        tab.dialog = None;
                    }
                    "Input.dragIntercepted" => tab.drag = Some(params["data"].clone()),
                    "Page.javascriptDialogOpening" => tab.dialog = Some(params),
                    "Page.javascriptDialogClosed" => tab.dialog = None,
                    "Network.requestWillBeSent" => push_event(tab, json!({
                        "kind": "request",
                        "id": params["requestId"],
                        "method": params["request"]["method"],
                        "url": params["request"]["url"],
                        "type": params["type"],
                        "headers": safe_headers(&params["request"]["headers"]),
                    })),
                    "Network.responseReceived" => push_event(tab, json!({
                        "kind": "response",
                        "id": params["requestId"],
                        "status": params["response"]["status"],
                        "mime": params["response"]["mimeType"],
                        "url": params["response"]["url"],
                        "headers": safe_headers(&params["response"]["headers"]),
                    })),
                    "Network.webSocketCreated" => push_event(tab, json!({"kind": "ws", "id": params["requestId"], "url": params["url"]})),
                    "Network.webSocketFrameSent" | "Network.webSocketFrameReceived" => push_event(tab, json!({
                        "kind": "ws-frame", "id": params["requestId"],
                        "direction": if method.ends_with("Sent") { "sent" } else { "received" },
                        "data": params["response"]["payloadData"].as_str().map(|data| data.chars().take(2000).collect::<String>()),
                    })),
                    "Network.eventSourceMessageReceived" => push_event(tab, json!({
                        "kind": "sse", "id": params["requestId"], "event": params["eventName"],
                        "data": params["data"].as_str().map(|data| data.chars().take(2000).collect::<String>()),
                    })),
                    "Network.loadingFailed" => push_event(tab, json!({
                        "kind": "failed",
                        "id": params["requestId"],
                        "error": params["errorText"],
                    })),
                    "Runtime.consoleAPICalled" => {
                        let text = params["args"]
                            .as_array()
                            .map(|args| {
                                args.iter()
                                    .map(|arg| match &arg["value"] {
                                        Value::String(text) => text.clone(),
                                        Value::Null => arg["description"]
                                            .as_str()
                                            .unwrap_or("")
                                            .to_string(),
                                        other => other.to_string(),
                                    })
                                    .collect::<Vec<_>>()
                                    .join(" ")
                            })
                            .unwrap_or_default();
                        push_event(tab, json!({
                            "kind": "console",
                            "level": params["type"],
                            "text": text.chars().take(500).collect::<String>(),
                        }));
                    }
                    "BackgroundService.backgroundServiceEventReceived" => {
                        let event = &params["backgroundServiceEvent"];
                        push_event(tab, json!({
                            "kind": "background",
                            "service": event["service"], "event": event["eventName"],
                            "origin": event["origin"], "instance": event["instanceId"],
                            "at": event["timestamp"],
                        }));
                    }
                    "ServiceWorker.workerRegistrationUpdated" => {
                        for registration in params["registrations"].as_array().into_iter().flatten() {
                            push_event(tab, json!({"kind": "sw", "what": "registration", "scope": registration["scopeURL"], "deleted": registration["isDeleted"]}));
                        }
                    }
                    "ServiceWorker.workerVersionUpdated" => {
                        for version in params["versions"].as_array().into_iter().flatten() {
                            push_event(tab, json!({"kind": "sw", "what": "version", "script": version["scriptURL"], "status": version["status"], "running": version["runningStatus"]}));
                        }
                    }
                    "ServiceWorker.workerErrorReported" => push_event(tab, json!({"kind": "sw", "what": "error", "message": params["errorMessage"]["errorMessage"]})),
                    "Tracing.dataCollected" => {
                        for item in params["value"].as_array().into_iter().flatten() {
                            push_event(tab, json!({"kind": "trace", "event": item}));
                        }
                    }
                    "Tracing.tracingComplete" => push_event(tab, json!({"kind": "trace-complete", "stream": params["stream"]})),
                    "Preload.prerenderStatusUpdated" => push_event(tab, json!({
                        "kind": "prerender", "url": params["key"]["url"], "status": params["status"],
                        "reason": params["prerenderStatus"], "detail": params["disallowedMojoInterface"],
                    })),
                    "Runtime.exceptionThrown" => push_event(tab, json!({
                        "kind": "exception",
                        "text": params["exceptionDetails"]["exception"]["description"]
                            .as_str()
                            .or(params["exceptionDetails"]["text"].as_str())
                            .unwrap_or("")
                            .chars()
                            .take(500)
                            .collect::<String>(),
                    })),
                    _ => {}
                }
            });
        }

        fn on_dev_tools_agent_detached(&self, browser: Option<&mut Browser>) {
            let Some(browser) = browser else {
                return;
            };
            let browser_id = browser.identifier();
            CDP.with(|cdp| {
                let mut state = cdp.borrow_mut();
                if let Some((_, tab)) = tab_for_browser(&mut state, browser_id) {
                    tab.enabled = false;
                    tab.contexts.clear();
                }
            });
        }
    }
}

/// Request or response headers without credentials.
fn safe_headers(headers: &Value) -> Value {
    let Some(map) = headers.as_object() else {
        return Value::Null;
    };
    Value::Object(
        map.iter()
            .map(|(name, value)| {
                let secret = matches!(
                    name.to_ascii_lowercase().as_str(),
                    "cookie" | "set-cookie" | "authorization" | "proxy-authorization"
                );
                (
                    name.clone(),
                    if secret {
                        json!("[hidden]")
                    } else {
                        value.clone()
                    },
                )
            })
            .collect(),
    )
}

fn push_event(tab: &mut TabCdp, event: Value) {
    if tab.events.len() >= EVENT_RING {
        tab.events.pop_front();
    }
    tab.events.push_back(event);
}

fn host_for(tab_id: &str) -> Option<BrowserHost> {
    super::state::content_view_for_tab(tab_id)
        .and_then(|view| view.browser())
        .and_then(|browser| browser.host())
}

/// Attaches the DevTools observer to a freshly created content browser and
/// installs the page agent for every future document.
pub fn attach(tab_id: &str, browser: &Browser) {
    let Some(host) = browser.host() else {
        return;
    };
    let mut observer = Observer::new();
    let registration = host.add_dev_tools_message_observer(Some(&mut observer));
    CDP.with(|cdp| {
        cdp.borrow_mut().tabs.insert(
            tab_id.to_string(),
            TabCdp {
                browser_id: browser.identifier(),
                _registration: registration,
                ..TabCdp::default()
            },
        );
    });
    enable(tab_id, &host);
    super::exec::spawn(super::fingerprint::install(tab_id.to_string()));
    super::exec::spawn(super::adblock::install_module(tab_id.to_string()));
}

/// The tabs that have a DevTools session.
pub fn tab_ids() -> Vec<String> {
    CDP.with(|cdp| cdp.borrow().tabs.keys().cloned().collect())
}

/// Enables the domains AgentGo listens to and installs the page agent for
/// every future document of this DevTools session.
fn enable(tab_id: &str, host: &BrowserHost) {
    CDP.with(|cdp| {
        if let Some(tab) = cdp.borrow_mut().tabs.get_mut(tab_id) {
            tab.enabled = true;
        }
    });
    let mut setup = vec![
        ("Page.enable", json!({})),
        ("Runtime.enable", json!({})),
        (
            "Page.addScriptToEvaluateOnNewDocument",
            json!({"source": AGENT_JS, "worldName": WORLD}),
        ),
    ];
    // The Chrome Web Store's own "Add to Chrome" installs into AgentGo.
    setup.push((
        "Page.addScriptToEvaluateOnNewDocument",
        json!({"source": include_str!("webstore.js")}),
    ));
    // The page agent reports to the browser through a function that exists
    // only in its own world; pages cannot call or see it.
    setup.push((
        "Runtime.addBinding",
        json!({"name": "__agReport", "executionContextName": WORLD}),
    ));
    if super::state::state().options.background() {
        // An agent-started window is never the active one, but its pages must
        // still behave as if their tab had the focus: `:focus` styles, blur
        // validation, `document.hasFocus()` and focus events all depend on it.
        setup.push((
            "Emulation.setFocusEmulationEnabled",
            json!({"enabled": true}),
        ));
    }
    for (method, params) in setup {
        let _ = send_to(host, method, params);
    }
}

/// Called by the load handler when a main-frame navigation starts. This is
/// the authoritative navigation signal: it does not depend on the DevTools
/// session surviving the navigation.
pub fn navigation_started(tab_id: &str) {
    CDP.with(|cdp| {
        if let Some(tab) = cdp.borrow_mut().tabs.get_mut(tab_id) {
            tab.generation += 1;
            tab.last_lines.clear();
            tab.contexts.clear();
            tab.dialog = None;
        }
    });
    super::sessions::emit("tab.navigated", json!({"tabId": tab_id}));
}

/// Forgets a tab that is about to close; its in-flight calls time out.
/// Resolves once the page's focus emulation is off. Chromium (152) hits a
/// CHECK and takes the whole browser down when a page is destroyed while its
/// focus is still emulated, so the caller must wait for this before it closes
/// the browser.
pub fn detach(tab_id: &str) -> impl std::future::Future<Output = ()> {
    let released = host_for(tab_id).and_then(|host| {
        send_to(
            &host,
            "Emulation.setFocusEmulationEnabled",
            json!({"enabled": false}),
        )
        .ok()
    });
    // The observer registration must outlive the wait, or the answer is lost.
    let tab = CDP.with(|cdp| cdp.borrow_mut().tabs.remove(tab_id));
    async move {
        if let Some(slot) = released {
            let _ = slot.wait(2_000).await;
        }
        drop(tab);
    }
}

thread_local! {
    /// When AgentGo last sent DevTools input (an agent's pointer or keys) to
    /// each tab. A click the person makes never comes this way.
    static LAST_INPUT: std::cell::RefCell<HashMap<String, std::time::Instant>> =
        std::cell::RefCell::new(HashMap::new());
}

/// Whether DevTools input reached this tab within the last `ms`
/// milliseconds: a user gesture then may have been an agent's, not the
/// person's.
pub fn agent_input_recent(tab_id: &str, ms: u64) -> bool {
    LAST_INPUT.with(|last| {
        last.borrow()
            .get(tab_id)
            .is_some_and(|at| at.elapsed() < std::time::Duration::from_millis(ms))
    })
}

fn send(tab_id: &str, method: &str, params: Value) -> Result<Slot<CdpResult>, String> {
    if method.starts_with("Input.") {
        LAST_INPUT.with(|last| {
            last.borrow_mut()
                .insert(tab_id.to_string(), std::time::Instant::now())
        });
    }
    let host = host_for(tab_id).ok_or_else(|| format!("tab {tab_id} has no browser"))?;
    let enabled = CDP.with(|cdp| cdp.borrow().tabs.get(tab_id).is_some_and(|tab| tab.enabled));
    if !enabled {
        enable(tab_id, &host);
    }
    send_to(&host, method, params)
}

fn send_to(host: &BrowserHost, method: &str, params: Value) -> Result<Slot<CdpResult>, String> {
    let slot = Slot::default();
    let id = CDP.with(|cdp| {
        let mut state = cdp.borrow_mut();
        state.next_id += 1;
        let id = state.next_id;
        state.pending.insert(id, slot.clone());
        id
    });
    let message = json!({"id": id, "method": method, "params": params}).to_string();
    if host.send_dev_tools_message(Some(message.as_bytes())) == 0 {
        CDP.with(|cdp| cdp.borrow_mut().pending.remove(&id));
        return Err(format!("DevTools rejected {method}"));
    }
    Ok(slot)
}

/// Calls one DevTools method on a tab and waits for its result.
pub async fn call(tab_id: &str, method: &str, params: Value) -> CdpResult {
    call_timeout(tab_id, method, params, 15_000).await
}

/// [`call`] with an explicit timeout in milliseconds.
pub async fn call_timeout(tab_id: &str, method: &str, params: Value, timeout_ms: u64) -> CdpResult {
    let slot = send(tab_id, method, params)?;
    slot.wait(timeout_ms)
        .await
        .unwrap_or_else(|| Err(format!("{method} timed out after {timeout_ms} ms")))
}

/// Sends several DevTools calls at once, then waits for each (results in
/// order): many small calls cost about one round trip instead of one each.
pub async fn call_all(tab_id: &str, calls: Vec<(&str, Value)>, timeout_ms: u64) -> Vec<CdpResult> {
    let sent: Vec<(String, Result<Slot<CdpResult>, String>)> = calls
        .into_iter()
        .map(|(method, params)| (method.to_string(), send(tab_id, method, params)))
        .collect();
    let mut out = Vec::with_capacity(sent.len());
    for (method, slot) in sent {
        out.push(match slot {
            Ok(slot) => slot
                .wait(timeout_ms)
                .await
                .unwrap_or_else(|| Err(format!("{method} timed out after {timeout_ms} ms"))),
            Err(error) => Err(error),
        });
    }
    out
}

/// Sends an input event and waits for its result, but stops waiting as soon
/// as the page opens a JavaScript dialog: the renderer is then blocked inside
/// the event handler and would only answer after the dialog is handled.
pub async fn input(tab_id: &str, method: &str, params: Value) -> Result<(), String> {
    let slot = send(tab_id, method, params)?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        if let Some(result) = slot.wait(20).await {
            return result.map(|_| ());
        }
        if dialog(tab_id).is_some() {
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            return Err(format!("{method} timed out after 15000 ms"));
        }
    }
}

async fn agent_context(tab_id: &str) -> Result<i64, String> {
    let known = CDP.with(|cdp| {
        let state = cdp.borrow();
        let tab = state.tabs.get(tab_id)?;
        tab.contexts.get(tab.main_frame.as_ref()?).copied()
    });
    if let Some(context) = known {
        return Ok(context);
    }
    // First use on a document that predates the observer (or the new-document
    // script has not run yet): create the world explicitly.
    // While a navigation waits for a site that has not answered, Chromium
    // holds DevTools messages for the page until the navigation commits.
    let tree = call_timeout(tab_id, "Page.getFrameTree", json!({}), 3_000)
        .await
        .map_err(|error| {
            if error.contains("timed out") {
                format!("{PENDING}: the site has not answered yet")
            } else {
                error
            }
        })?;
    let frame = tree["frameTree"]["frame"]["id"]
        .as_str()
        .ok_or("page has no main frame")?
        .to_string();
    let world = call(
        tab_id,
        "Page.createIsolatedWorld",
        json!({"frameId": frame, "worldName": WORLD}),
    )
    .await?;
    let context = world["executionContextId"]
        .as_i64()
        .ok_or("isolated world was not created")?;
    call(
        tab_id,
        "Runtime.evaluate",
        json!({"expression": AGENT_JS, "contextId": context}),
    )
    .await?;
    CDP.with(|cdp| {
        if let Some(tab) = cdp.borrow_mut().tabs.get_mut(tab_id) {
            tab.main_frame = Some(frame.clone());
            tab.contexts.insert(frame, context);
        }
    });
    Ok(context)
}

fn unwrap_evaluation(result: Value) -> CdpResult {
    if let Some(details) = result.get("exceptionDetails") {
        return Err(details["exception"]["description"]
            .as_str()
            .or(details["text"].as_str())
            .unwrap_or("script failed")
            .lines()
            .next()
            .unwrap_or("script failed")
            .to_string());
    }
    Ok(result["result"]["value"].clone())
}

/// Whether DevTools answers for the page within `ms` (it does not while a
/// navigation waits for a site that has not answered).
pub async fn readable(tab_id: &str, ms: u64) -> bool {
    call_timeout(tab_id, "Page.getFrameTree", json!({}), ms)
        .await
        .is_ok()
}

/// Start of the error for a page that cannot be read while its navigation
/// is still waiting for the site.
pub const PENDING: &str = "the page is not ready";

/// Evaluates an expression in the isolated AgentGo world of the main frame.
/// A context lost to a concurrent navigation is retried once.
pub async fn agent_eval(tab_id: &str, expression: &str, timeout_ms: u64) -> CdpResult {
    if super::state::tab_is_loading(tab_id) && !readable(tab_id, 1_500).await {
        return Err(format!("{PENDING}: the site has not answered yet"));
    }
    let mut last = String::new();
    for _ in 0..2 {
        let context = agent_context(tab_id).await?;
        let result = call_timeout(
            tab_id,
            "Runtime.evaluate",
            json!({
                "expression": expression,
                "contextId": context,
                "returnByValue": true,
                "awaitPromise": true,
            }),
            timeout_ms,
        )
        .await;
        match result {
            Ok(value) => return unwrap_evaluation(value),
            Err(error) if error.contains("context") || error.contains("navigat") => {
                CDP.with(|cdp| {
                    if let Some(tab) = cdp.borrow_mut().tabs.get_mut(tab_id) {
                        tab.contexts.clear();
                    }
                });
                last = error;
                super::exec::sleep(60).await;
            }
            Err(error) => return Err(error),
        }
    }
    Err(format!("page context unavailable: {last}"))
}

/// Evaluates in the isolated world and returns a remote object id (used to
/// hand a DOM node to DevTools methods such as `DOM.setFileInputFiles`).
pub async fn agent_object(tab_id: &str, expression: &str) -> Result<String, String> {
    let context = agent_context(tab_id).await?;
    let result = call(
        tab_id,
        "Runtime.evaluate",
        json!({"expression": expression, "contextId": context}),
    )
    .await?;
    result["result"]["objectId"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| "element reference is no longer in the document".to_string())
}

/// [`agent_object`] with the remote objects kept in `group`, so everything
/// derived from them is freed with one `Runtime.releaseObjectGroup`.
pub async fn agent_object_in(
    tab_id: &str,
    expression: &str,
    group: &str,
) -> Result<String, String> {
    // A page whose site has not answered has no document to ask yet: say so
    // at once, as agent_eval does, instead of waiting out the call.
    if super::state::tab_is_loading(tab_id) && !readable(tab_id, 1_500).await {
        return Err(format!("{PENDING}: the site has not answered yet"));
    }
    let context = agent_context(tab_id).await?;
    let result = call_timeout(
        tab_id,
        "Runtime.evaluate",
        json!({"expression": expression, "contextId": context, "objectGroup": group}),
        5_000,
    )
    .await?;
    result["result"]["objectId"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| "the page returned nothing".to_string())
}

/// Evaluates in the page's own (main) world. Policy gates this upstream.
pub async fn page_eval(tab_id: &str, expression: &str, timeout_ms: u64) -> CdpResult {
    let result = call_timeout(
        tab_id,
        "Runtime.evaluate",
        json!({"expression": expression, "returnByValue": true, "awaitPromise": true}),
        timeout_ms,
    )
    .await?;
    unwrap_evaluation(result)
}

/// Main-frame navigation counter for a tab.
pub fn generation(tab_id: &str) -> u64 {
    CDP.with(|cdp| {
        cdp.borrow()
            .tabs
            .get(tab_id)
            .map_or(0, |tab| tab.generation)
    })
}

/// Takes the data of a drag the page started since the last call.
pub fn take_drag(tab_id: &str) -> Option<Value> {
    CDP.with(|cdp| {
        cdp.borrow_mut()
            .tabs
            .get_mut(tab_id)
            .and_then(|tab| tab.drag.take())
    })
}

/// Pending JavaScript dialog, if one is blocking the tab.
pub fn dialog(tab_id: &str) -> Option<Value> {
    CDP.with(|cdp| {
        cdp.borrow()
            .tabs
            .get(tab_id)
            .and_then(|tab| tab.dialog.clone())
    })
}

/// Drains up to `limit` recorded network/console events of one kind set.
pub fn events(tab_id: &str, kinds: &[&str], limit: usize, clear: bool) -> Vec<Value> {
    CDP.with(|cdp| {
        let mut state = cdp.borrow_mut();
        let Some(tab) = state.tabs.get_mut(tab_id) else {
            return Vec::new();
        };
        let matches = |event: &Value| {
            event["kind"]
                .as_str()
                .is_some_and(|kind| kinds.contains(&kind))
        };
        let mut picked: Vec<Value> = tab.events.iter().filter(|e| matches(e)).cloned().collect();
        if picked.len() > limit {
            picked.drain(..picked.len() - limit);
        }
        if clear {
            tab.events.retain(|event| !matches(event));
        }
        picked
    })
}

/// Reads and replaces the per-tab snapshot baseline used for deltas.
pub fn swap_lines(tab_id: &str, lines: Vec<String>) -> Vec<String> {
    CDP.with(|cdp| {
        cdp.borrow_mut()
            .tabs
            .get_mut(tab_id)
            .map(|tab| std::mem::replace(&mut tab.last_lines, lines))
            .unwrap_or_default()
    })
}

/// Current element-reference floor for the tab.
pub fn ref_base(tab_id: &str) -> u64 {
    CDP.with(|cdp| cdp.borrow().tabs.get(tab_id).map_or(0, |tab| tab.ref_base))
}

/// Raises the element-reference floor after a snapshot.
pub fn raise_ref_base(tab_id: &str, value: u64) {
    CDP.with(|cdp| {
        if let Some(tab) = cdp.borrow_mut().tabs.get_mut(tab_id) {
            tab.ref_base = tab.ref_base.max(value);
        }
    });
}
