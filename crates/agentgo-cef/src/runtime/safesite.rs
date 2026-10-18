//! Dangerous-site protection for the pages tabs open: known malware and
//! phishing sites (offline lists) and addresses that imitate well-known sites
//! (look-alike letters, a brand's domain as a subdomain, a brand next to a
//! lure word). Everything is decided on this computer; no address is sent
//! anywhere.
//!
//! A blocked page is replaced by a warning. The person may open it anyway
//! with a real click on that warning; an agent never can (it gets an error
//! and must hand the page to the person), and while an agent is connected the
//! warning offers no way through.

use std::collections::HashSet;
use std::sync::Mutex;

use agentgo_adblock::threats::{self, Threat};

use super::{sessions, state};

/// Hosts the person chose to open anyway, for this run of the browser.
static ALLOWED: Mutex<Option<HashSet<String>>> = Mutex::new(None);

/// Why a page was stopped.
struct Verdict {
    /// Error code shown on the warning page.
    code: &'static str,
    /// The imitated brand, for look-alikes.
    brand: Option<&'static str>,
    /// Short reason for logs and agents.
    what: String,
}

fn enabled() -> bool {
    state::settings_snapshot()["dangerousSiteProtection"] != false
}

fn verdict(url: &str) -> Option<Verdict> {
    if let Some(threat) = threats::threat_for_url(url) {
        return Some(match threat {
            Threat::Malware => Verdict {
                code: "AGENTGO_DANGEROUS_MALWARE",
                brand: None,
                what: "a known malware site".into(),
            },
            Threat::Phishing => Verdict {
                code: "AGENTGO_DANGEROUS_PHISHING",
                brand: None,
                what: "a known phishing site".into(),
            },
        });
    }
    let hit = threats::lookalike_for_url(url)?;
    Some(Verdict {
        code: "AGENTGO_DECEPTIVE_SITE",
        brand: Some(hit.brand),
        what: format!("a site imitating {} ({})", hit.brand, hit.reason),
    })
}

fn host(url: &str) -> String {
    url.split("://")
        .nth(1)
        .and_then(|rest| rest.split(['/', '?', '#']).next())
        .map(|authority| {
            authority
                .rsplit('@')
                .next()
                .unwrap_or(authority)
                .to_ascii_lowercase()
        })
        .unwrap_or_default()
}

fn encode(value: &str) -> String {
    value
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Decides a main-frame navigation of a content tab (UI thread). Returns the
/// warning page to show instead, or `None` to let it load.
pub fn check(tab: &str, target: &str, frame_url: &str, user_gesture: bool) -> Option<String> {
    if !enabled() {
        return None;
    }
    let verdict = verdict(target)?;
    let site = host(target);
    // The person's "open anyway" is for the person: a tab an agent holds or
    // just drove still gets the warning.
    let agent_tab =
        sessions::lease_holder(tab).is_some() || super::cdp::agent_input_recent(tab, 5_000);
    if !agent_tab
        && ALLOWED
            .lock()
            .ok()
            .is_some_and(|allowed| allowed.as_ref().is_some_and(|a| a.contains(&site)))
    {
        return None;
    }
    let agent = sessions::agent_present();
    // "Open anyway": a real click on this very page's warning, by the person.
    let warning = state::error_page_url(target, verdict.code);
    let warning_head = warning.split("&theme=").next().unwrap_or(&warning);
    // An agent's DevTools clicks count as gestures too: refuse any that came
    // with agent input, even from an agent that disconnected first.
    let person = user_gesture && !agent && !super::cdp::agent_input_recent(tab, 5_000);
    if person && frame_url.starts_with(warning_head) {
        if let Ok(mut allowed) = ALLOWED.lock() {
            allowed
                .get_or_insert_with(HashSet::new)
                .insert(site.clone());
        }
        state::log_event(&format!("AG-SECURITY dangerous-site-opened-anyway {site}"));
        return None;
    }
    state::log_event(&format!(
        "AG-SECURITY dangerous-site-blocked {site} ({})",
        verdict.what
    ));
    state::set_load_error(
        tab,
        &format!(
            "{}: AgentGo stopped {} before it opened. Do not continue; if the person needs this page, hand it to them.",
            verdict.code, verdict.what
        ),
    );
    Some(format!(
        "{warning}&brand={}&agent={}",
        encode(verdict.brand.unwrap_or("")),
        u8::from(agent)
    ))
}

/// Query parameters that only exist to track a click across sites. They are
/// removed from the address before the page is requested.
const TRACKING_PARAMS: &[&str] = &[
    "fbclid",
    "gclid",
    "gclsrc",
    "dclid",
    "gbraid",
    "wbraid",
    "msclkid",
    "mc_eid",
    "igshid",
    "igsh",
    "yclid",
    "ymclid",
    "twclid",
    "ttclid",
    "li_fat_id",
    "epik",
    "_hsenc",
    "_hsmi",
    "__hssc",
    "__hstc",
    "__hsfp",
    "hsctatracking",
    "mkt_tok",
    "oly_anon_id",
    "oly_enc_id",
    "vero_conv",
    "vero_id",
    "wickedid",
    "rb_clickid",
    "ss_email_id",
    "_openstat",
    "srsltid",
    "s_kwcid",
    "ef_id",
    "irclickid",
    "utm_source",
    "utm_medium",
    "utm_campaign",
    "utm_term",
    "utm_content",
    "utm_id",
    "utm_name",
    "utm_reader",
    "utm_social",
    "utm_brand",
    "utm_place",
    "utm_pubreferrer",
    "utm_swu",
];

/// The address without its tracking parameters, or `None` when it has none.
pub fn without_tracking(url: &str) -> Option<String> {
    if state::settings_snapshot()["stripTracking"] == false {
        return None;
    }
    let (head, fragment) = match url.split_once('#') {
        Some((head, fragment)) => (head, Some(fragment)),
        None => (url, None),
    };
    let (base, query) = head.split_once('?')?;
    let mut removed = false;
    let kept: Vec<&str> = query
        .split('&')
        .filter(|pair| {
            let name = pair.split('=').next().unwrap_or("").to_ascii_lowercase();
            let tracking = TRACKING_PARAMS.contains(&name.as_str());
            removed |= tracking;
            !tracking && !pair.is_empty()
        })
        .collect();
    if !removed {
        return None;
    }
    let mut clean = base.to_string();
    if !kept.is_empty() {
        clean.push('?');
        clean.push_str(&kept.join("&"));
    }
    if let Some(fragment) = fragment {
        clean.push('#');
        clean.push_str(fragment);
    }
    Some(clean)
}

/// What the protection covers (`security.status`).
pub fn status() -> serde_json::Value {
    serde_json::json!({
        "enabled": enabled(),
        "knownDangerousHosts": threats::threat_count(),
        "lookalikeDetection": true,
        "sources": ["URLhaus (malware)", "uBlock Badware risks", "Phishing URL Blocklist (OpenPhish, PhishTank, IPThreat)"],
        "private": "checked on this computer; no address is sent anywhere",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clean(url: &str) -> Option<String> {
        // The settings store is not running in unit tests; the default is on.
        without_tracking(url)
    }

    #[test]
    fn tracking_parameters_are_removed_and_the_rest_kept() {
        assert_eq!(
            clean("https://shop.example/item?id=7&utm_source=mail&fbclid=abc#reviews").as_deref(),
            Some("https://shop.example/item?id=7#reviews")
        );
        assert_eq!(
            clean("https://example.com/?gclid=1").as_deref(),
            Some("https://example.com/")
        );
        assert_eq!(
            clean("https://example.com/?UTM_Campaign=x&q=a").as_deref(),
            Some("https://example.com/?q=a")
        );
        assert_eq!(clean("https://example.com/?q=utm_source"), None);
        assert_eq!(clean("https://example.com/search?q=a&page=2"), None);
        assert_eq!(clean("https://example.com/"), None);
    }
}
