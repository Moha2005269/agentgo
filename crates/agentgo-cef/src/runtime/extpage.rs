//! `agentgo-ext://<id>/`: an extension's own pages (its toolbar pop-up and
//! options page), served from the extension's folder.
//!
//! Each extension gets its own origin, separate from AgentGo's trusted
//! `agentgo://shell` (which answers the command bridge only for its own
//! documents) and from every other extension. Pages get the `chrome.*` shim
//! (extpage.js on top of extshim.js) before their own scripts, with the same
//! storage the extension's content scripts use. Only the extension's own
//! pages may use the `__agentgo/` endpoints; a website that tries to load
//! them, or the extension's files, from its own page is refused.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use cef::*;
use serde_json::{json, Value};

use super::scheme::plain_asset;
use super::{exec, state};

/// The tab a pop-up was opened over (`chrome.tabs.query` answers with it).
static ACTIVE_TAB: Mutex<Option<Value>> = Mutex::new(None);

/// Tabs AgentGo opened for an extension's page: tab id -> extension id.
static PAGE_TABS: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());

/// Records that `tab` shows `extension`'s own page, so its requests (the
/// engine's early preloads included, which come with no frame or referrer)
/// are the extension's.
pub fn register_page_tab(tab: &str, extension: &str) {
    if let Ok(mut all) = PAGE_TABS.lock() {
        all.retain(|(t, _)| t != tab);
        all.push((tab.to_string(), extension.to_string()));
    }
}

/// A main-frame navigation of `tab` to `target`: a pop-up tab that leaves
/// its extension's pages is an ordinary tab from then on (a website there
/// must not get the extension's endpoints or storage).
pub fn note_navigation(tab: &str, target: &str) {
    if target.starts_with("about:") {
        return;
    }
    if let Ok(mut all) = PAGE_TABS.lock() {
        all.retain(|(t, id)| {
            t != tab
                || target.starts_with(&format!("agentgo-ext://{id}/"))
                || target.starts_with(&format!("chrome-extension://{id}/"))
        });
    }
}

/// Whether a frame of `tab` may load `target`, an extension page: only in the
/// tab AgentGo opened for that extension, or from that extension's own pages.
/// A website (or an agent through one) cannot open them.
pub fn navigation_allowed(tab: &str, frame_url: &str, target: &str) -> bool {
    let Some((id, _)) = split(target) else {
        return false;
    };
    page_tab_of(tab).as_deref() == Some(id.as_str())
        || frame_url.starts_with(&format!("agentgo-ext://{id}/"))
        || frame_url.starts_with(&format!("chrome-extension://{id}/"))
}

fn page_tab_of(tab: &str) -> Option<String> {
    PAGE_TABS
        .lock()
        .ok()?
        .iter()
        .find(|(t, _)| t == tab)
        .map(|(_, e)| e.clone())
}

/// The extension whose page `tab` shows (a tab AgentGo opened for it).
pub fn page_extension(tab: &str) -> Option<String> {
    page_tab_of(tab)
}

/// The tab the person was on when they opened the last pop-up.
pub fn active_tab() -> Value {
    ACTIVE_TAB
        .lock()
        .ok()
        .and_then(|slot| slot.clone())
        .unwrap_or_else(|| json!({}))
}

/// Remembers the tab the person was on when they opened a pop-up.
pub fn set_active_tab(tab: Value) {
    if let Ok(mut slot) = ACTIVE_TAB.lock() {
        *slot = Some(tab);
    }
}

fn mime(path: &str) -> &'static str {
    match path
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "wasm" => "application/wasm",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// `agentgo-ext://<id>/<path>` (or an engine-run extension's
/// `chrome-extension://<id>/<path>`) split into id and path (without query).
fn split(url: &str) -> Option<(String, String)> {
    let rest = url
        .strip_prefix("agentgo-ext://")
        .or_else(|| url.strip_prefix("chrome-extension://"))?;
    let (id, path) = rest.split_once('/').unwrap_or((rest, ""));
    let path = path.split(['?', '#']).next().unwrap_or("").to_string();
    Some((id.to_ascii_lowercase(), path))
}

/// A file inside the extension's folder, never outside it.
fn inside(folder: &Path, relative: &str) -> Option<PathBuf> {
    if relative.is_empty()
        || relative
            .split('/')
            .any(|seg| seg == ".." || seg.starts_with('.'))
    {
        return None;
    }
    let path = folder.join(relative).canonicalize().ok()?;
    path.starts_with(folder.canonicalize().ok()?)
        .then_some(path)
}

fn query_value(url: &str, key: &str) -> Option<String> {
    let query = url.split_once('?')?.1;
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k == key).then(|| super::scheme::percent_decode(v))
    })
}

/// The `<script>` that loads the shim, placed before the page's own scripts.
fn with_shim(html: &[u8]) -> Vec<u8> {
    let text = String::from_utf8_lossy(html);
    let tag = r#"<script src="/__agentgo/shim.js"></script>"#;
    let lower = text.to_ascii_lowercase();
    let at = lower
        .find("<head")
        .and_then(|start| lower[start..].find('>').map(|end| start + end + 1))
        .unwrap_or(0);
    format!("{}{tag}{}", &text[..at], &text[at..]).into_bytes()
}

fn shim(id: &str, manifest: &Value, messages: &Value, storage: &Value) -> String {
    let tab = ACTIVE_TAB
        .lock()
        .ok()
        .and_then(|slot| slot.clone())
        .unwrap_or(Value::Null);
    format!(
        "({})({},{},{},{},{},{});",
        include_str!("extpage.js").trim_end().trim_end_matches(';'),
        include_str!("extshim.js").trim_end().trim_end_matches(';'),
        json!(id),
        manifest,
        messages,
        storage,
        tab
    )
}

/// Answers one request for an extension's page or file.
fn respond(
    url: &str,
    tab: Option<&str>,
    frame_url: &str,
    referrer: &str,
    frame_is_main: bool,
) -> cef::ResourceHandler {
    let refuse = || plain_asset(b"not found".to_vec(), 404, "text/plain");
    let Some((id, path)) = split(url) else {
        return refuse();
    };
    // The extension's own pages may use its files and endpoints. Opening one
    // of its pages in a tab or window (a top-level navigation) is fine too;
    // a website loading them from its own page is not.
    // While a page first loads its frame can still report about:blank; the
    // referrer of its requests is the page itself.
    let base = format!("agentgo-ext://{id}/");
    let own_page = tab.and_then(page_tab_of).as_deref() == Some(id.as_str())
        || frame_url.starts_with(&base)
        || referrer.starts_with(&base);
    let navigation = frame_is_main && mime(&path).starts_with("text/html");
    if !own_page && !navigation {
        state::log_event(&format!(
            "AG-SECURITY extension-page-refused {id}/{path} from {} referrer {}",
            frame_url.chars().take(80).collect::<String>(),
            referrer.chars().take(80).collect::<String>()
        ));
        return refuse();
    }
    let Some((folder, manifest, messages, storage)) = super::extensions::page_parts(&id) else {
        return refuse();
    };
    match path.as_str() {
        "__agentgo/shim.js" if own_page => plain_asset(
            shim(&id, &manifest, &messages, &storage).into_bytes(),
            200,
            "text/javascript; charset=utf-8",
        ),
        "__agentgo/storage" if own_page => {
            // The id is the page's own, whatever the payload says.
            if let Some(mut change) =
                query_value(url, "p").and_then(|p| serde_json::from_str::<Value>(&p).ok())
            {
                change["id"] = json!(id);
                let payload = change.to_string();
                exec::post(move || super::extensions::storage_from_page(&payload));
            }
            plain_asset(b"{}".to_vec(), 200, "application/json")
        }
        "__agentgo/open" if own_page => {
            if let Some(target) = query_value(url, "u")
                .filter(|u| u.starts_with("https://") || u.starts_with("http://"))
            {
                exec::post(move || {
                    let _ = state::create_content_tab(&target);
                });
            }
            plain_asset(b"{}".to_vec(), 200, "application/json")
        }
        _ => {
            let Some(file) = inside(&folder, &path) else {
                return refuse();
            };
            match std::fs::read(&file) {
                Ok(bytes) if mime(&path).starts_with("text/html") => {
                    plain_asset(with_shim(&bytes), 200, mime(&path))
                }
                Ok(bytes) => plain_asset(bytes, 200, mime(&path)),
                Err(_) => refuse(),
            }
        }
    }
}

cef::wrap_scheme_handler_factory! {
    pub(crate) struct ExtensionPageFactory;

    impl SchemeHandlerFactory {
        fn create(
            &self,
            browser: Option<&mut cef::Browser>,
            frame: Option<&mut cef::Frame>,
            _scheme_name: Option<&cef::CefString>,
            request: Option<&mut cef::Request>,
        ) -> Option<cef::ResourceHandler> {
            let request = request?;
            let tab = browser.and_then(|b| state::tab_for_browser_id(b.identifier()));
            let url = CefString::from(&request.url()).to_string();
            let referrer = CefString::from(&request.referrer_url()).to_string();
            let frame_is_main = frame.as_ref().is_some_and(|f| f.is_main() != 0);
            let frame_url = frame.map(|f| CefString::from(&f.url()).to_string()).unwrap_or_default();
            Some(respond(&url, tab.as_deref(), &frame_url, &referrer, frame_is_main))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shim_goes_before_the_page_scripts() {
        assert_eq!(
            String::from_utf8(with_shim(
                b"<!doctype html><html><HEAD lang=x><script src=p.js></script>"
            ))
            .unwrap(),
            r#"<!doctype html><html><HEAD lang=x><script src="/__agentgo/shim.js"></script><script src=p.js></script>"#
        );
        assert!(String::from_utf8(with_shim(b"<div>no head</div>"))
            .unwrap()
            .starts_with("<script src=\"/__agentgo/shim.js\">"));
    }

    #[test]
    fn addresses_and_files_stay_inside_the_extension() {
        assert_eq!(
            split("agentgo-ext://Abc/popup/index.html?x=1#y"),
            Some(("abc".into(), "popup/index.html".into()))
        );
        let root = std::env::temp_dir().join(format!("agentgo-extpage-{}", std::process::id()));
        std::fs::create_dir_all(root.join("popup")).unwrap();
        std::fs::write(root.join("popup").join("index.html"), "x").unwrap();
        assert!(inside(&root, "popup/index.html").is_some());
        assert!(inside(&root, "../secret").is_none());
        assert!(inside(&root, ".git/config").is_none());
        assert!(inside(&root, "").is_none());
        std::fs::remove_dir_all(&root).ok();
        assert_eq!(
            query_value(
                "agentgo-ext://a/__agentgo/open?u=https%3A%2F%2Fx.test%2F",
                "u"
            )
            .as_deref(),
            Some("https://x.test/")
        );
        assert_eq!(mime("a/B.PNG"), "image/png");
    }
}
