//! Ad and tracker blocking: the engine, the on/off switch and the handler
//! that cancels matching subresource requests of content tabs.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use agentgo_adblock::youtube::{Kind, Rules};
use agentgo_adblock::AdBlocker;
use cef::*;
use serde_json::json;

use super::{cdp, state};

static YOUTUBE_RULES: RwLock<Option<Arc<Rules>>> = RwLock::new(None);

fn adblock_dir() -> std::path::PathBuf {
    state::data_dir().join("adblock")
}

/// The YouTube rules: a newer `youtube-rules.json` in the data folder's
/// `adblock` directory replaces the bundled ones, so they can be updated
/// without a new browser build.
fn youtube_rules() -> Arc<Rules> {
    if let Some(rules) = YOUTUBE_RULES.read().ok().and_then(|r| r.clone()) {
        return rules;
    }
    reload_youtube_rules()
}

fn reload_youtube_rules() -> Arc<Rules> {
    let rules = std::fs::read_to_string(adblock_dir().join("youtube-rules.json"))
        .ok()
        .and_then(|text| Rules::from_json(&text))
        .unwrap_or_else(Rules::bundled);
    let rules = Arc::new(rules);
    if let Ok(mut slot) = YOUTUBE_RULES.write() {
        *slot = Some(rules.clone());
    }
    rules
}

/// The YouTube module script (an updated copy in the data folder wins).
fn youtube_module() -> String {
    std::fs::read_to_string(adblock_dir().join("youtube-module.js"))
        .unwrap_or_else(|_| Rules::bundled_module().to_string())
}

const MODULE_WORLD: &str = "AgentGoYoutube";

thread_local! {
    static MODULES: std::cell::RefCell<std::collections::HashMap<String, String>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// Installs (or removes, when blocking is off) the YouTube module in a tab.
/// It lives in its own isolated world: it sees the page's DOM but the page
/// cannot see it, its variables or its timers.
pub async fn install_module(tab: String) {
    let previous = MODULES.with(|m| m.borrow_mut().remove(&tab));
    if let Some(identifier) = previous {
        let _ = cdp::call(
            &tab,
            "Page.removeScriptToEvaluateOnNewDocument",
            json!({"identifier": identifier}),
        )
        .await;
    }
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    reload_youtube_rules();
    let source = youtube_module();
    if let Ok(out) = cdp::call(
        &tab,
        "Page.addScriptToEvaluateOnNewDocument",
        json!({"source": source, "worldName": MODULE_WORLD}),
    )
    .await
    {
        if let Some(identifier) = out["identifier"].as_str() {
            MODULES.with(|m| m.borrow_mut().insert(tab, identifier.to_string()));
        }
    }
}

fn refresh_modules() {
    for tab in cdp::tab_ids() {
        super::exec::spawn(install_module(tab));
    }
}

struct FilterState {
    kind: Kind,
    input: Vec<u8>,
    output: Option<Vec<u8>>,
    sent: usize,
}

cef::wrap_response_filter! {
    struct YoutubeFilter {
        state: Arc<Mutex<FilterState>>,
    }

    impl ResponseFilter {
        fn init_filter(&self) -> ::std::os::raw::c_int {
            1
        }

        fn filter(
            &self,
            data_in: Option<&mut Vec<u8>>,
            data_in_read: Option<&mut usize>,
            data_out: Option<&mut Vec<u8>>,
            data_out_written: Option<&mut usize>,
        ) -> ResponseFilterStatus {
            let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(input) = data_in {
                // The whole body is read first: the ad data sits anywhere in
                // it and the JSON has to be parsed as one document.
                state.input.extend_from_slice(input);
                if let Some(read) = data_in_read {
                    *read = input.len();
                }
                if let Some(written) = data_out_written {
                    *written = 0;
                }
                return ResponseFilterStatus::NEED_MORE_DATA;
            }
            if state.output.is_none() {
                let body = std::mem::take(&mut state.input);
                let rewritten = youtube_rules().rewrite(state.kind, &body).unwrap_or(body);
                state.output = Some(rewritten);
            }
            let Some(out) = data_out else {
                return ResponseFilterStatus::NEED_MORE_DATA;
            };
            let sent = state.sent;
            let all = state.output.as_deref().unwrap_or_default();
            let count = (all.len() - sent).min(out.len());
            out[..count].copy_from_slice(&all[sent..sent + count]);
            let finished = sent + count >= all.len();
            state.sent += count;
            if let Some(written) = data_out_written {
                *written = count;
            }
            if finished {
                ResponseFilterStatus::DONE
            } else {
                ResponseFilterStatus::NEED_MORE_DATA
            }
        }
    }
}

cef::wrap_resource_request_handler! {
    struct YoutubeHandler {
        kind: Kind,
    }

    impl ResourceRequestHandler {
        // The default answer of this callback is "cancel".
        fn on_before_resource_load(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            request: Option<&mut Request>,
            _callback: Option<&mut Callback>,
        ) -> ReturnValue {
            if let Some(request) = request {
                add_privacy_signal(request);
            }
            ReturnValue::CONTINUE
        }

        fn resource_response_filter(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            _request: Option<&mut Request>,
            response: Option<&mut Response>,
        ) -> Option<ResponseFilter> {
            let response = response?;
            if response.status() != 200 {
                return None;
            }
            let mime = CefString::from(&response.mime_type()).to_string().to_lowercase();
            let wanted = match self.kind {
                Kind::Json => mime.contains("json"),
                Kind::Html => mime.contains("html"),
            };
            wanted.then(|| {
                YoutubeFilter::new(Arc::new(Mutex::new(FilterState {
                    kind: self.kind,
                    input: Vec::new(),
                    output: None,
                    sent: 0,
                })))
            })
        }
    }
}

static ENGINE: OnceLock<AdBlocker> = OnceLock::new();
/// Mirrors the `adBlock` setting so the per-request check never locks or
/// clones the settings document.
static ENABLED: AtomicBool = AtomicBool::new(true);

/// Reads the `adBlock` setting: on unless it is explicitly `false`.
pub fn apply(settings: &serde_json::Value) {
    let on = settings["adBlock"] != false;
    if ENABLED.swap(on, Ordering::Relaxed) != on {
        super::exec::post(refresh_modules);
    }
}

static STARTED: AtomicBool = AtomicBool::new(false);

/// Builds the engine on its own thread. Web requests wait for it (see
/// `engine`), so nothing slips through while it loads.
pub fn start(settings: &serde_json::Value) {
    apply(settings);
    STARTED.store(true, Ordering::Relaxed);
    let _ = std::thread::Builder::new()
        .name("agentgo-adblock".into())
        .spawn(|| {
            let began = std::time::Instant::now();
            let _ = ENGINE.set(AdBlocker::bundled());
            state::log_event(&format!(
                "AG-RUNTIME adblock-ready ms={}",
                began.elapsed().as_millis()
            ));
        });
}

/// The engine. A web request that arrives while it is still loading (the
/// first page of a launch) waits for it, on the network thread, instead of
/// passing unfiltered; the wait is bounded so a failed load cannot hang.
fn engine() -> Option<&'static AdBlocker> {
    if let Some(blocker) = ENGINE.get() {
        return Some(blocker);
    }
    if !STARTED.load(Ordering::Relaxed) {
        return None;
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(5));
        if let Some(blocker) = ENGINE.get() {
            return Some(blocker);
        }
    }
    None
}

/// What the blocker is doing right now, for `security.status`: whether it is
/// switched on, whether the engine has loaded (until then requests wait for
/// it), and what has been blocked so far.
pub fn status() -> serde_json::Value {
    let lists = agentgo_adblock::bundled_list_versions();
    let rules = youtube_rules();
    json!({
        "active": ENABLED.load(Ordering::Relaxed) && ENGINE.get().is_some(),
        "enabled": ENABLED.load(Ordering::Relaxed),
        "engineReady": ENGINE.get().is_some(),
        "blockedRequests": ENGINE.get().map_or(0, AdBlocker::blocked_count),
        "lists": lists.iter().map(|(name, version)| json!({"name": name, "version": version})).collect::<Vec<_>>(),
        "cosmeticFiltering": true,
        "youtube": {
            "networkRewrite": ENABLED.load(Ordering::Relaxed),
            "module": ENABLED.load(Ordering::Relaxed),
            "rulesFromDataFolder": adblock_dir().join("youtube-rules.json").is_file(),
            "hosts": rules.hosts(),
        },
    })
}

/// Adds the live numbers the Settings panel shows to a `settings.get` reply.
/// They are never stored in the settings file.
pub fn decorate(settings: &mut serde_json::Value) {
    if let Some(map) = settings.as_object_mut() {
        let blocked = ENGINE.get().map_or(0, AdBlocker::blocked_count);
        map.insert("adBlockBlocked".into(), blocked.into());
        map.insert("adBlockReady".into(), ENGINE.get().is_some().into());
    }
}

/// Hides ad elements and runs the anti-adblock scriptlets in a frame that
/// just started loading. `scripts` is false for the second pass at the end of
/// the load, which only repeats the (idempotent) styles.
pub fn inject(frame: &mut Frame, scripts: bool) {
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    let Some(blocker) = ENGINE.get() else { return };
    let url = CefString::from(&frame.url()).to_string();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return;
    }
    let cosmetic = blocker.cosmetic(&url, frame.is_main() != 0);
    if cosmetic.is_empty() {
        return;
    }
    let css = serde_json::Value::String(cosmetic.css);
    // Constructed style sheets are not subject to a page's content security
    // policy, unlike a <style> element; the element is the fallback. The
    // scriptlets go in as plain code, not through eval, for the same reason.
    // "Already applied" is remembered under a hidden, per-run symbol, not a
    // named global a page could look for.
    let mark = format!(
        "(()=>{{const k=Symbol.for('{}');return window[k]||(Object.defineProperty(window,k,{{value:{{}}}}),window[k])}})()",
        page_marker()
    );
    let mut code = format!(
        "(function(css,m){{try{{if(!css||m.c)return;\
         const put=()=>{{if(m.c)return true;\
         try{{const sheet=new CSSStyleSheet();sheet.replaceSync(css);\
         document.adoptedStyleSheets=[...document.adoptedStyleSheets,sheet];m.c=1;return true}}catch(e){{}}\
         const parent=document.head||document.documentElement;if(!parent)return false;\
         const style=document.createElement('style');style.textContent=css;parent.appendChild(style);m.c=1;return true}};\
         if(!put()){{new MutationObserver((o2,o)=>{{if(put())o.disconnect()}}).observe(document,{{childList:true,subtree:true}})}}\
         }}catch(e){{}}}})({css},{mark});"
    );
    if scripts && !cosmetic.script.is_empty() && !is_youtube(&url) {
        code.push_str(&format!("if(!{mark}.s){{{mark}.s=1;try{{\n"));
        code.push_str(&cosmetic.script);
        code.push_str("\n}catch(e){}}");
    }
    frame.execute_java_script(
        Some(&CefString::from(code.as_str())),
        Some(&CefString::from(url.as_str())),
        0,
    );
}

/// A random name for this run's hidden page marker.
pub fn page_marker() -> &'static str {
    static MARKER: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    MARKER.get_or_init(|| {
        let mut bytes = [0u8; 8];
        let _ = ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut bytes);
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    })
}

// Lets the request through with the Global Privacy Control signal
// (`Sec-GPC: 1`): the person does not consent to their data being sold or
// shared, which several laws (CCPA, CPA) make binding.
cef::wrap_resource_request_handler! {
    struct PrivacySignalHandler;

    impl ResourceRequestHandler {
        fn on_before_resource_load(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            request: Option<&mut Request>,
            _callback: Option<&mut Callback>,
        ) -> ReturnValue {
            if let Some(request) = request {
                add_privacy_signal(request);
            }
            ReturnValue::CONTINUE
        }
    }
}

fn add_privacy_signal(request: &mut Request) {
    request.set_header_by_name(
        Some(&CefString::from("Sec-GPC")),
        Some(&CefString::from("1")),
        1,
    );
}

cef::wrap_resource_request_handler! {
    struct CancelHandler;

    impl ResourceRequestHandler {
        fn on_before_resource_load(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            _request: Option<&mut Request>,
            _callback: Option<&mut Callback>,
        ) -> ReturnValue {
            ReturnValue::CANCEL
        }
    }
}

/// A handler that cancels the request when the filter lists say so, and
/// `None` (CEF carries on as usual) otherwise. Top-level navigations are
/// never cancelled; the engine itself refuses them.
pub fn handler_for(
    request: &mut Request,
    initiator: Option<&CefString>,
) -> Option<ResourceRequestHandler> {
    let url = CefString::from(&request.url()).to_string();
    if !["http:", "https:", "ws:", "wss:"]
        .iter()
        .any(|scheme| url.starts_with(scheme))
    {
        return None;
    }
    if !ENABLED.load(Ordering::Relaxed) {
        return Some(PrivacySignalHandler::new());
    }
    let Some(blocker) = engine() else {
        return Some(PrivacySignalHandler::new());
    };
    let page = initiator
        .map(ToString::to_string)
        .filter(|origin| !origin.is_empty())
        .unwrap_or_else(|| CefString::from(&request.referrer_url()).to_string());
    if blocker.should_block(&url, &page, kind(request.resource_type())) {
        return Some(CancelHandler::new());
    }
    Some(
        youtube_rules()
            .kind_of(&url)
            .map(YoutubeHandler::new)
            .unwrap_or_else(PrivacySignalHandler::new),
    )
}

/// Whether a page URL is YouTube's, where network-level removal replaces the
/// filter lists' page scriptlets (those change the page's own objects, which
/// is what its ad-blocker check looks for).
fn is_youtube(url: &str) -> bool {
    youtube_rules().covers(url)
}

/// CEF's resource type in the vocabulary of filter lists.
fn kind(resource: ResourceType) -> &'static str {
    let is = |other: ResourceType| resource == other;
    if is(ResourceType::MAIN_FRAME) || is(ResourceType::NAVIGATION_PRELOAD_MAIN_FRAME) {
        "main_frame"
    } else if is(ResourceType::SUB_FRAME) || is(ResourceType::NAVIGATION_PRELOAD_SUB_FRAME) {
        "sub_frame"
    } else if is(ResourceType::STYLESHEET) {
        "stylesheet"
    } else if is(ResourceType::SCRIPT)
        || is(ResourceType::WORKER)
        || is(ResourceType::SHARED_WORKER)
        || is(ResourceType::SERVICE_WORKER)
    {
        "script"
    } else if is(ResourceType::IMAGE) || is(ResourceType::FAVICON) {
        "image"
    } else if is(ResourceType::FONT_RESOURCE) {
        "font"
    } else if is(ResourceType::OBJECT) || is(ResourceType::PLUGIN_RESOURCE) {
        "object"
    } else if is(ResourceType::MEDIA) {
        "media"
    } else if is(ResourceType::XHR) {
        "xhr"
    } else if is(ResourceType::PING) {
        "ping"
    } else if is(ResourceType::CSP_REPORT) {
        "csp_report"
    } else {
        "other"
    }
}
