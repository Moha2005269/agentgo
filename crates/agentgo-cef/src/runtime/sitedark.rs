//! Dark mode for websites, separate from the browser's own light/dark theme.
//!
//! The recolouring engine is Dark Reader (MIT, `darkreader.js`, see
//! third_party/licenses). It runs inside the page, so it only ever sees what
//! the page itself shows, and only for the sites the person has not excluded.
//! Settings: `siteDark` (bool), `siteDarkBrightness` / `siteDarkContrast`
//! (percent) and `siteDarkExclude` (a list of host names).

use cef::{CefString, Frame, ImplBrowser, ImplBrowserView, ImplFrame};
use serde_json::Value;

const ENGINE: &str = include_str!("darkreader.js");

fn host_of(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let host = rest.split(['/', '?', '#']).next()?;
    let host = host.rsplit('@').next()?.split(':').next()?;
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

/// Whether `host` (or a parent domain of it) is on the exclusion list.
pub fn excluded(settings: &Value, host: &str) -> bool {
    settings["siteDarkExclude"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .any(|entry| host == entry || host.ends_with(&format!(".{entry}")))
}

fn percent(settings: &Value, key: &str) -> i64 {
    let value = &settings[key];
    value
        .as_i64()
        .or_else(|| value.as_f64().map(|n| n.round() as i64))
        .or_else(|| value.as_str().and_then(|s| s.trim().parse().ok()))
        .unwrap_or(100)
        .clamp(50, 150)
}

fn enable_call(settings: &Value) -> String {
    format!(
        "DarkReader.enable({{brightness:{},contrast:{},sepia:0}});",
        percent(settings, "siteDarkBrightness"),
        percent(settings, "siteDarkContrast")
    )
}

/// The script that turns the engine on for a page, loading it first when needed.
fn enable_script(settings: &Value) -> String {
    format!(
        "(function(){{try{{if(!window.DarkReader){{\n{ENGINE}\n}}{}}}catch(e){{}}}})();",
        enable_call(settings)
    )
}

const DISABLE: &str = "try{window.DarkReader&&DarkReader.disable()}catch(e){}";

/// Called as a page starts loading (and again for its frames).
pub fn inject(frame: &mut Frame, settings: &Value) {
    if settings["siteDark"] != true || frame.is_main() == 0 {
        return;
    }
    let url = CefString::from(&frame.url()).to_string();
    let Some(host) = host_of(&url) else { return };
    if excluded(settings, &host) {
        return;
    }
    frame.execute_java_script(
        Some(&CefString::from(enable_script(settings).as_str())),
        Some(&CefString::from(url.as_str())),
        0,
    );
}

/// Settings changed: bring every open page in line without a reload.
pub fn refresh(settings: &Value) {
    for id in super::state::content_tab_ids() {
        let Some(frame) = super::state::content_view_for_tab(&id)
            .and_then(|view| view.browser())
            .and_then(|browser| browser.main_frame())
        else {
            continue;
        };
        let url = CefString::from(&frame.url()).to_string();
        let Some(host) = host_of(&url) else { continue };
        let script = if settings["siteDark"] == true && !excluded(settings, &host) {
            // The engine keeps the first brightness it was given until it is restarted.
            format!("{DISABLE}\n{}", enable_script(settings))
        } else {
            DISABLE.to_string()
        };
        frame.execute_java_script(
            Some(&CefString::from(script.as_str())),
            Some(&CefString::from(url.as_str())),
            0,
        );
    }
}

/// The page address's host, for the per-site toggle.
pub fn host(url: &str) -> Option<String> {
    host_of(url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn hosts_and_exclusions() {
        assert_eq!(
            host_of("https://User@Docs.Example.com:8080/a?b").as_deref(),
            Some("docs.example.com")
        );
        assert_eq!(host_of("agentgo://shell/tools.html"), None);
        let settings = json!({"siteDarkExclude": ["example.com"]});
        assert!(excluded(&settings, "docs.example.com"));
        assert!(excluded(&settings, "example.com"));
        assert!(!excluded(&settings, "notexample.com"));
    }
}
