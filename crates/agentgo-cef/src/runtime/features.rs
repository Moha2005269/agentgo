//! Panels that sit on top of the browser rather than inside a page: the
//! "Report a problem" bundle and the dark-mode-per-site toggle. (Import and
//! the setup wizard live in `onboarding.rs`.)
//!
//! Everything here is reachable only from the trusted tools document.

use std::io::Write;

use serde_json::{json, Value};

use super::sessions::{Caller, Fail, Reply};
use super::{cdp, dispatch, exec, state};

/// Where a filled-in problem report goes. Taken from the workspace's
/// `repository` field; a placeholder address means no tracker is set up yet
/// and the report is only saved to disk.
fn tracker_url() -> Option<String> {
    let repository = env!("CARGO_PKG_REPOSITORY").trim_end_matches('/');
    repository
        .starts_with("https://github.com/")
        .then(|| format!("{repository}/issues/new"))
}

/// URL-encodes `text` for a query string.
fn encode(text: &str) -> String {
    text.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

/// Cuts the query string and fragment off every web address in `text`; they
/// are where tokens and search words live.
fn strip_queries(text: &str) -> String {
    text.split_inclusive(char::is_whitespace)
        .map(|word| {
            let core = word.trim_end();
            if core.starts_with("http://") || core.starts_with("https://") {
                let cut = core.find(['?', '#']).unwrap_or(core.len());
                format!("{}{}", &core[..cut], &word[core.len()..])
            } else {
                word.to_string()
            }
        })
        .collect()
}

/// Everything that leaves the machine goes through here: web-address queries
/// dropped, the Windows user name removed from paths, then the shared secret
/// redaction (`credential.redact`).
fn scrub(text: &str) -> (String, usize) {
    let mut text = strip_queries(text);
    let mut hidden = 0;
    for var in ["USERPROFILE", "USERNAME"] {
        if let Ok(value) = std::env::var(var) {
            if value.len() > 2 {
                let placeholder = if var == "USERPROFILE" {
                    "%USERPROFILE%"
                } else {
                    "<user>"
                };
                let before = text.len();
                text = text.replace(&value, placeholder);
                hidden += usize::from(text.len() != before);
            }
        }
    }
    let (text, count) = super::misc::redact(&text);
    (text, count + hidden)
}

fn base64_decode(text: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let (mut bits, mut have) = (0u32, 0u32);
    for byte in text.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => continue,
        };
        bits = (bits << 6) | u32::from(value);
        have += 6;
        if have >= 8 {
            have -= 8;
            out.push((bits >> have) as u8);
            bits &= (1 << have) - 1;
        }
    }
    out
}

/// The report text: versions, a few non-secret settings, the page's address
/// without its query, and the browser's recent event log (the trace).
async fn compose(tab: &str) -> String {
    let version = dispatch::dispatch(&Caller::shell(), "browser.version", json!({}))
        .await
        .unwrap_or_else(|_| json!({}));
    let settings = state::settings_snapshot();
    let kept: Vec<String> = [
        "theme",
        "motion",
        "adBlock",
        "siteDark",
        "fingerprintProtection",
        "controlMode",
        "agentTabs",
        "hardwareAcceleration",
        "searchEngine",
    ]
    .iter()
    .map(|key| format!("  {key}: {}", settings[*key]))
    .collect();
    let events = state::recent_events(150);
    format!(
        "AgentGo problem report\n\
         AgentGo {} / Chromium {} / protocol {}\n\
         Windows {} ({})\n\
         Render mode: {}\n\
         Page: {}\n\
         Open tabs: {}\n\
         Settings:\n{}\n\n\
         Recent events (newest last):\n{}\n",
        version["agentgo"].as_str().unwrap_or("?"),
        // Dotted digits would be scrubbed as a long number.
        version["chromium"]
            .as_str()
            .unwrap_or("?")
            .replace('.', "_"),
        version["protocol"].as_str().unwrap_or("?"),
        std::env::var("OS").unwrap_or_default(),
        std::env::consts::ARCH,
        state::render_mode(),
        state::tab_url(tab),
        state::content_tab_ids().len(),
        kept.join("\n"),
        events.join("\n"),
    )
}

/// A picture of the page the problem is on, as base64 JPEG (None when the
/// page cannot be captured).
async fn screenshot(tab: &str) -> Option<String> {
    if !state::tab_url(tab).starts_with("http") {
        return None;
    }
    let shot = cdp::call(
        tab,
        "Page.captureScreenshot",
        json!({"format": "jpeg", "quality": 55}),
    )
    .await
    .ok()?;
    shot["data"].as_str().map(str::to_string)
}

/// `ui.report_collect`: builds the redacted bundle for the person to review.
pub async fn report_collect() -> Reply {
    let tab = state::active_tab_id();
    let (text, redactions) = scrub(&compose(&tab).await);
    let image = screenshot(&tab).await;
    Ok(json!({
        "text": text,
        "redactions": redactions,
        "screenshot": image,
        "tracker": tracker_url().is_some(),
    }))
}

/// `ui.report_submit`: saves the reviewed bundle as a zip, shows it in File
/// Explorer and opens a pre-filled issue on the tracker (if one is set up).
pub async fn report_submit(params: &Value) -> Reply {
    let tab = state::active_tab_id();
    let note = params["description"].as_str().unwrap_or("").trim();
    let include_shot = params["includeScreenshot"] == true;
    let (body, redactions) = scrub(&compose(&tab).await);
    let (note, _) = scrub(note);
    let image = if include_shot {
        screenshot(&tab).await
    } else {
        None
    };
    let dir = state::download_dir();
    let path = dir.join(format!(
        "AgentGo-report-{}.zip",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs())
    ));
    let report = format!("{note}\n\n{body}");
    let zip_path = path.clone();
    let saved = exec::blocking(15_000, move || -> Result<(), String> {
        std::fs::create_dir_all(zip_path.parent().ok_or("no folder")?)
            .map_err(|e| e.to_string())?;
        let file = std::fs::File::create(&zip_path).map_err(|e| e.to_string())?;
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("report.txt", options)
            .map_err(|e| e.to_string())?;
        zip.write_all(report.as_bytes())
            .map_err(|e| e.to_string())?;
        if let Some(image) = image {
            zip.start_file("screenshot.jpg", options)
                .map_err(|e| e.to_string())?;
            zip.write_all(&base64_decode(&image))
                .map_err(|e| e.to_string())?;
        }
        zip.finish().map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
    .ok_or_else(|| Fail::internal("saving the report timed out"))?;
    saved.map_err(Fail::internal)?;
    // Show the file so it can be dragged into the issue.
    let _ = std::process::Command::new("explorer.exe")
        .arg(format!("/select,{}", path.display()))
        .spawn();
    let mut issue = Value::Null;
    if let Some(base) = tracker_url() {
        let title = if note.is_empty() {
            "Problem report".to_string()
        } else {
            note.lines().next().unwrap_or("").chars().take(80).collect()
        };
        let short: String = format!(
            "{note}\n\n<!-- The full redacted report is in {} (attach it here). -->\n\n```\n{}\n```",
            path.file_name().and_then(|n| n.to_str()).unwrap_or("the zip"),
            body.chars().take(3_500).collect::<String>()
        );
        let url = format!(
            "{base}?title={}&body={}",
            encode(&format!("[bug] {title}")),
            encode(&short)
        );
        state::create_content_tab(&url).map_err(Fail::internal)?;
        issue = json!(url.len());
    }
    Ok(json!({
        "path": path.to_string_lossy(),
        "redactions": redactions,
        "issueOpened": !issue.is_null(),
    }))
}

/// `ui.site_dark`: turns dark mode on or off for the page's site (the list of
/// excluded sites in settings), or everywhere with `{all: bool}`.
pub fn site_dark(params: &Value) -> Reply {
    let settings = state::settings_snapshot();
    if let Some(all) = params["all"].as_bool() {
        state::update_settings(&json!({"siteDark": all})).map_err(Fail::params)?;
        return Ok(json!({"siteDark": all}));
    }
    let tab = state::active_tab_id();
    let host = super::sitedark::host(&state::tab_url(&tab))
        .ok_or_else(|| Fail::params("only web pages can be darkened"))?;
    let mut list: Vec<String> = settings["siteDarkExclude"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();
    let dark_here = settings["siteDark"] == true && !super::sitedark::excluded(&settings, &host);
    if dark_here {
        list.push(host.clone());
    } else {
        list.retain(|entry| host != *entry && !host.ends_with(&format!(".{entry}")));
    }
    list.sort();
    list.dedup();
    let turn_on = !dark_here;
    state::update_settings(&json!({
        "siteDark": settings["siteDark"] == true || turn_on,
        "siteDarkExclude": list,
    }))
    .map_err(Fail::params)?;
    Ok(json!({"host": host, "dark": turn_on}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queries_are_cut_from_addresses() {
        let text = strip_queries("see https://a.example/p?token=1#x and more\nhttps://b.example/");
        assert_eq!(text, "see https://a.example/p and more\nhttps://b.example/");
    }

    #[test]
    fn base64_round_trip() {
        assert_eq!(base64_decode("aGVsbG8="), b"hello");
        assert_eq!(base64_decode("AAEC"), vec![0, 1, 2]);
    }

    #[test]
    fn urls_are_encoded() {
        assert_eq!(encode("a b&c"), "a%20b%26c");
    }
}
