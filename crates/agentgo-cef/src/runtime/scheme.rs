//! `agentgo://` scheme: bundled UI assets and the trusted command bridge.
//!
//! The browser chrome, the tools panel and the New Tab page are served from
//! the bundled UI with strict CSPs. `__bridge` requests are answered on the
//! CEF UI thread by the same dispatcher that serves agent transports, and
//! only for the exact trusted document that issued them.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use agentgo_protocol::ErrorCode;
use cef::*;
use serde_json::{json, Value};

use super::sessions::{Caller, Fail};
use super::{dispatch, exec, state};

/// Resolves a request path inside `agentgo://shell/...` to a bundled
/// file, refusing traversal.
fn resolve_file(ui_root: &std::path::Path, path: &str) -> Option<(PathBuf, &'static str)> {
    let path = path.split(['?', '#']).next().unwrap_or("");
    let rel = match path {
        "" | "/" | "index.html" => "index.html",
        other => other.trim_start_matches('/'),
    };
    if rel
        .split('/')
        .any(|seg| seg == ".." || seg.is_empty() || seg.starts_with('.'))
    {
        return None;
    }
    let mime = match rel.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js") | Some("mjs") => "text/javascript; charset=utf-8",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        Some("ico") => "image/x-icon",
        _ => return None,
    };
    Some((ui_root.join(rel), mime))
}

/// Strict CSP for every bundled AgentGo document: same-origin scripts only,
/// no network access except the local bridge. Favicons of visited sites are
/// the one remote resource the chrome may display.
fn shell_csp() -> &'static str {
    "default-src 'self'; \
     script-src 'self'; \
     style-src 'self' 'unsafe-inline'; \
     font-src 'self'; \
     img-src 'self' data: https: http:; \
     connect-src 'self' agentgo://shell; \
     frame-ancestors 'none'; \
     base-uri 'none'"
}

/// Which bundled document issued a bridge request.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Document {
    /// The tab strip / toolbar. Full operator authority.
    Chrome,
    /// History, downloads, settings and agent panels. Full operator authority.
    Tools,
    /// The New Tab page. It lives in the content context, so it only gets a
    /// read-only summary and nothing else.
    NewTab,
}

fn trusted_document(frame_url: &str, frame_is_main: bool) -> Option<Document> {
    if !frame_is_main || !agentgo_shell_bridge::is_trusted_shell_url(frame_url) {
        return None;
    }
    match frame_url
        .strip_prefix("agentgo://shell/")
        .and_then(|path| path.split(['?', '#']).next())
    {
        Some("chrome.html") => Some(Document::Chrome),
        Some("tools.html") => Some(Document::Tools),
        Some("index.html") | Some("") => Some(Document::NewTab),
        _ => None,
    }
}

/// Percent-decodes a query-string value.
pub(super) fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
                if let Ok(v) = u8::from_str_radix(hex, 16) {
                    out.push(v);
                    i += 3;
                } else {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn http_status(code: ErrorCode) -> i32 {
    match code {
        ErrorCode::Unauthorized | ErrorCode::PolicyDenied => 403,
        ErrorCode::MethodNotFound | ErrorCode::CapabilityUnsupported => 404,
        ErrorCode::InvalidEnvelope | ErrorCode::InvalidParams => 400,
        _ => 500,
    }
}

/// Runs one bridge command on the UI thread.
async fn bridge_run(document: Document, method: String, params: Value) -> (i32, String) {
    let reply = match (document, method.as_str()) {
        (Document::NewTab, "newtab.state") => Ok(state::newtab_state()),
        (Document::NewTab, _) => Err(Fail::new(
            ErrorCode::Unauthorized,
            "the New Tab page may only read newtab.state",
        )),
        (Document::Chrome | Document::Tools, "ui.open_tool") => match params["tool"].as_str() {
            Some(tool) => state::open_tool(tool).map_err(Fail::params),
            None => Err(Fail::params("ui.open_tool requires {tool}")),
        },
        (Document::Tools, "ui.close_tool") => Ok(state::close_tool()),
        (Document::Tools, "ui.run_action") => state::run_action(
            params["name"].as_str().unwrap_or(""),
            params["section"].as_str().unwrap_or(""),
        )
        .map_err(Fail::params),
        (Document::Tools, "ui.report_collect") => super::features::report_collect().await,
        (Document::Tools, "ui.report_submit") => super::features::report_submit(&params).await,
        (Document::Tools, "ui.site_dark") => super::features::site_dark(&params),
        (Document::Tools, "ui.import_scan") => super::onboarding::scan().await,
        (Document::Tools, "ui.import_run") => super::onboarding::run(&params).await,
        (Document::Tools, "ui.setup_done") => super::onboarding::done(),
        (Document::Tools, "ui.permission_list") => Ok(super::permission::list()),
        (Document::Tools, "ui.permission_revoke") => super::permission::revoke(&params)
            .await
            .map_err(Fail::params),
        (Document::Tools, "ui.permission_answer") => {
            super::permission::answer(&params).map_err(Fail::params)
        }
        (Document::Tools, "ui.context_action") => {
            state::context_action(&params).map_err(Fail::params)
        }
        (Document::Chrome, "ui.suggest") => {
            state::show_suggestions(
                params["x"].as_i64().unwrap_or(240) as i32,
                params["width"].as_i64().unwrap_or(640) as i32,
                &params["items"],
                params["selected"].as_i64().unwrap_or(-1),
                params["text"].as_str().unwrap_or(""),
            );
            Ok(json!({"shown": params["items"].as_array().map_or(0, Vec::len)}))
        }
        (Document::Chrome, "ui.siteinfo") => {
            state::show_site_info(params["x"].as_i64().unwrap_or(16) as i32);
            Ok(json!({"accepted": true}))
        }
        (Document::Chrome, "ui.prompt") => {
            super::winmgr::person_prompt(params["text"].as_str().unwrap_or(""));
            Ok(json!({"sent": true}))
        }
        (Document::Tools, "ui.theme_origin") => {
            state::set_theme_origin(
                params["x"].as_f64().unwrap_or(0.0) as i32,
                params["y"].as_f64().unwrap_or(0.0) as i32,
            );
            Ok(json!({"accepted": true}))
        }
        (Document::Tools, "ui.find") => state::find_in_page(
            params["text"].as_str().unwrap_or(""),
            params["forward"] != false,
            params["matchCase"] == true,
            params["next"] == true,
        )
        .map(|()| json!({"accepted": true}))
        .map_err(Fail::internal),
        (Document::Chrome | Document::Tools, "ui.print") => {
            state::print_active_tab();
            Ok(json!({"accepted": true}))
        }
        (Document::Tools, "ui.open_download_folder") => {
            state::open_download_folder().map_err(Fail::internal)
        }
        (
            _,
            "ui.close_tool"
            | "ui.find"
            | "ui.open_download_folder"
            | "ui.context_action"
            | "ui.run_action"
            | "ui.report_collect"
            | "ui.report_submit"
            | "ui.site_dark"
            | "ui.import_scan"
            | "ui.import_run"
            | "ui.setup_done"
            | "ui.permission_answer"
            | "ui.permission_list"
            | "ui.permission_revoke",
        ) => Err(Fail::new(
            ErrorCode::Unauthorized,
            "browser UI method is not allowed from this document",
        )),
        _ => dispatch::dispatch(&Caller::shell(), &method, params).await,
    };
    match reply {
        Ok(result) => (200, json!({"ok": true, "result": result}).to_string()),
        Err(fail) => (
            http_status(fail.code),
            json!({"ok": false, "error": {"code": fail.code.as_str(), "message": fail.message, "data": fail.data}})
                .to_string(),
        ),
    }
}

/// A CEF callback moved to the UI thread and back.
struct SendCallback(Callback);
// SAFETY: CefCallback is reference counted with atomic counts and `cont` may
// be invoked from any thread.
unsafe impl Send for SendCallback {}

type Shared<T> = Arc<Mutex<T>>;

// In-memory handler for bundled assets and bridge replies. It sets the MIME
// type, charset and headers explicitly (custom schemes must not rely on
// sniffing) and owns its body, so nothing outlives the request.
cef::wrap_resource_handler! {
    struct MemoryHandler {
        // (status, mime, body): filled immediately for assets and by the
        // UI thread for bridge calls.
        response: Shared<Option<(i32, String, Vec<u8>)>>,
        // Pending bridge call, taken by open().
        job: Shared<Option<(Document, String, Value)>>,
        offset: Shared<usize>,
        csp: Option<String>,
    }

    impl ResourceHandler {
        fn open(
            &self,
            _request: Option<&mut Request>,
            handle_request: Option<&mut i32>,
            callback: Option<&mut Callback>,
        ) -> i32 {
            let job = self.job.lock().ok().and_then(|mut job| job.take());
            let (Some((document, method, params)), Some(callback)) = (job, callback) else {
                if let Some(handle_request) = handle_request {
                    *handle_request = 1;
                }
                return 1;
            };
            // Bridge call: answer later, from the UI thread.
            if let Some(handle_request) = handle_request {
                *handle_request = 0;
            }
            let response = self.response.clone();
            let callback = SendCallback(callback.clone());
            exec::post(move || {
                exec::spawn(async move {
                    let (status, body) = bridge_run(document, method, params).await;
                    if let Ok(mut slot) = response.lock() {
                        *slot = Some((status, "application/json".into(), body.into_bytes()));
                    }
                    let callback = callback;
                    callback.0.cont();
                });
            });
            state::wake_ui();
            1
        }

        fn response_headers(
            &self,
            response: Option<&mut Response>,
            response_length: Option<&mut i64>,
            _redirect_url: Option<&mut CefString>,
        ) {
            let Some(response) = response else {
                return;
            };
            let guard = self.response.lock().ok();
            let (status, mime, length) = guard
                .as_ref()
                .and_then(|slot| slot.as_ref())
                .map(|(status, mime, body)| (*status, mime.clone(), body.len()))
                .unwrap_or((500, "text/plain".into(), 0));
            let mime = mime.split(';').next().unwrap_or("text/plain").to_string();
            response.set_status(status);
            response.set_status_text(Some(&CefString::from(if status == 200 { "OK" } else { "Error" })));
            response.set_mime_type(Some(&CefString::from(mime.as_str())));
            let textual = mime.starts_with("text/") || mime == "application/json" || mime == "image/svg+xml";
            let content_type = if textual { format!("{mime}; charset=utf-8") } else { mime };
            let header = |name: &str, value: &str| {
                response.set_header_by_name(Some(&CefString::from(name)), Some(&CefString::from(value)), 1);
            };
            header("Content-Type", &content_type);
            if textual {
                response.set_charset(Some(&CefString::from("UTF-8")));
            }
            if let Some(csp) = &self.csp {
                header("Content-Security-Policy", csp);
            }
            header("Cache-Control", "no-store");
            header("X-Content-Type-Options", "nosniff");
            if let Some(response_length) = response_length {
                *response_length = length as i64;
            }
        }

        #[allow(clippy::not_unsafe_ptr_arg_deref)]
        fn read(
            &self,
            data_out: *mut u8,
            bytes_to_read: i32,
            bytes_read: Option<&mut i32>,
            _callback: Option<&mut ResourceReadCallback>,
        ) -> i32 {
            let Some(bytes_read) = bytes_read else {
                return 0;
            };
            *bytes_read = 0;
            let (Ok(guard), Ok(mut offset)) = (self.response.lock(), self.offset.lock()) else {
                return 0;
            };
            let Some((_, _, body)) = guard.as_ref() else {
                return 0;
            };
            if bytes_to_read <= 0 || *offset >= body.len() {
                return 0;
            }
            let amount = (bytes_to_read as usize).min(body.len() - *offset);
            // CEF supplies a writable buffer of at least bytes_to_read bytes.
            unsafe {
                std::ptr::copy_nonoverlapping(body.as_ptr().add(*offset), data_out, amount);
            }
            *offset += amount;
            *bytes_read = amount as i32;
            1
        }
    }
}

fn asset(body: Vec<u8>, status: i32, mime: &str) -> cef::ResourceHandler {
    MemoryHandler::new(
        Arc::new(Mutex::new(Some((status, mime.to_string(), body)))),
        Arc::new(Mutex::new(None)),
        Arc::new(Mutex::new(0)),
        Some(shell_csp().to_string()),
    )
}

/// A response with no CSP of AgentGo's own: an extension's page keeps the
/// policy its manifest and markup give it.
pub(super) fn plain_asset(body: Vec<u8>, status: i32, mime: &str) -> cef::ResourceHandler {
    MemoryHandler::new(
        Arc::new(Mutex::new(Some((status, mime.to_string(), body)))),
        Arc::new(Mutex::new(None)),
        Arc::new(Mutex::new(0)),
        None,
    )
}

fn bridge(document: Document, method: String, params: Value) -> cef::ResourceHandler {
    MemoryHandler::new(
        Arc::new(Mutex::new(None)),
        Arc::new(Mutex::new(Some((document, method, params)))),
        Arc::new(Mutex::new(0)),
        None,
    )
}

cef::wrap_scheme_handler_factory! {
    pub(crate) struct ShellSchemeFactoryImpl {
        ui_root: Arc<PathBuf>,
    }

    impl SchemeHandlerFactory {
        fn create(
            &self,
            _browser: Option<&mut cef::Browser>,
            frame: Option<&mut cef::Frame>,
            _scheme_name: Option<&cef::CefString>,
            request: Option<&mut cef::Request>,
        ) -> Option<cef::ResourceHandler> {
            let request = request?;
            let url = CefString::from(&request.url()).to_string();
            let path = url
                .strip_prefix("agentgo://shell")
                .unwrap_or("/")
                .trim_start_matches('/');
            let frame_is_main = frame.as_ref().map(|f| f.is_main() != 0).unwrap_or(false);
            let frame_url = frame
                .map(|f| CefString::from(&f.url()).to_string())
                .unwrap_or_default();

            if path.starts_with("__bridge") {
                let refuse = |code: ErrorCode, message: &str| {
                    let body = json!({"ok": false, "error": {"code": code.as_str(), "message": message}});
                    Some(asset(body.to_string().into_bytes(), http_status(code), "application/json"))
                };
                let Some(document) = trusted_document(&frame_url, frame_is_main) else {
                    state::log_event(&format!(
                        "AG-BRIDGE refused-untrusted-origin frame={:?} main={frame_is_main} path={:?}",
                        frame_url.chars().take(120).collect::<String>(),
                        path.split('&').next().unwrap_or("")
                    ));
                    return refuse(ErrorCode::Unauthorized, "bridge requires a trusted AgentGo browser UI document");
                };
                let query = path.split_once('?').map(|(_, query)| query).unwrap_or("");
                let mut method = None;
                let mut params = Value::Null;
                for pair in query.split('&') {
                    match pair.split_once('=') {
                        Some(("method", value)) => method = Some(percent_decode(value)),
                        Some(("params", value)) => {
                            params = serde_json::from_str(&percent_decode(value)).unwrap_or(Value::Null)
                        }
                        _ => {}
                    }
                }
                let Some(method) = method else {
                    return refuse(ErrorCode::InvalidEnvelope, "missing method");
                };
                return Some(bridge(document, method, params));
            }

            let Some((file, mime)) = resolve_file(&self.ui_root, path) else {
                return Some(asset(b"not found".to_vec(), 404, "text/plain"));
            };
            match std::fs::read(&file) {
                Ok(bytes) => Some(asset(bytes, 200, mime)),
                Err(error) => {
                    state::log_event(&format!("AG-SCHEME 404-read {path} {error}"));
                    Some(asset(b"not found".to_vec(), 404, "text/plain"))
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_exact_bundled_documents_are_trusted() {
        assert!(
            trusted_document("agentgo://shell/chrome.html?v=1", true) == Some(Document::Chrome)
        );
        assert!(trusted_document("agentgo://shell/tools.html", true) == Some(Document::Tools));
        assert!(
            trusted_document("agentgo://shell/index.html?embedded=1", true)
                == Some(Document::NewTab)
        );
        assert!(trusted_document("agentgo://shell/chrome.html", false).is_none());
        assert!(trusted_document("https://example.com/chrome.html", true).is_none());
        assert!(trusted_document("agentgo://shell/other.html", true).is_none());
        assert!(trusted_document("agentgo://evil/chrome.html", true).is_none());
    }

    #[test]
    fn asset_paths_cannot_escape_the_bundle() {
        let root = PathBuf::from("ui");
        assert!(resolve_file(&root, "../secret.html").is_none());
        assert!(resolve_file(&root, "a/../../x.js").is_none());
        assert!(resolve_file(&root, ".git/config.json").is_none());
        assert!(resolve_file(&root, "app.exe").is_none());
        assert!(resolve_file(&root, "fonts/manrope-latin.woff2").is_some());
    }
}
