//! Ad and tracker blocking for AgentGo.
//!
//! Wraps Brave's `adblock` engine (MPL-2.0) around a precompiled set of filter
//! lists (EasyList, EasyPrivacy, uBlock Origin's lists, AdGuard's base,
//! social-media and DNS lists, Peter Lowe's list and URLhaus) that is compiled
//! into the binary, so blocking works offline and the browser never contacts a
//! list server on its own. Refresh the lists and rebuild the snapshot with
//! `scripts\update-adblock-lists.ps1` before a release.
//!
//! Two questions are answered here: should this subresource request be
//! cancelled ([`AdBlocker::should_block`]), and what should be hidden or
//! patched on this page ([`AdBlocker::cosmetic`]). The crate knows nothing
//! about CEF; the runtime maps CEF's request fields onto it.

pub mod threats;
pub mod youtube;

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;

use adblock::lists::{FilterSet, ParseOptions};
use adblock::request::Request;
use adblock::resources::Resource;
use adblock::url_parser::parse_url;
use adblock::Engine;

const ENGINE_SNAPSHOT: &[u8] = include_bytes!("../data/engine.dat");
const RESOURCES: &str = include_str!("../data/resources.json");
const GENERIC_SELECTORS: &str = include_str!("../data/generic.txt");
const VERSIONS: &str = include_str!("../data/versions.txt");

/// A compiled filter engine plus a per-site exception list and a counter.
pub struct AdBlocker {
    engine: Engine,
    allowed_sites: RwLock<HashSet<String>>,
    blocked: AtomicU64,
}

/// What to do to a page besides cancelling requests.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Cosmetic {
    /// Style rules that hide page elements.
    pub css: String,
    /// Scriptlets (anti-adblock and tracker defusers) to run in the page.
    pub script: String,
}

impl Cosmetic {
    /// Whether there is nothing to apply.
    pub fn is_empty(&self) -> bool {
        self.css.is_empty() && self.script.is_empty()
    }
}

/// Compiles filter list texts into the snapshot the browser ships.
pub fn compile_snapshot<'a>(lists: impl IntoIterator<Item = &'a str>) -> Vec<u8> {
    build_engine(lists).serialize()
}

fn build_engine<'a>(lists: impl IntoIterator<Item = &'a str>) -> Engine {
    let mut set = FilterSet::new(false);
    for list in lists {
        set.add_filter_list(list.to_string(), ParseOptions::default());
    }
    Engine::new_with_filter_set(set)
}

/// Hide rules for elements named by a class or an id on any site: the plain
/// `##.banner` and `##\#top-ad` lines of the lists, minus the generic
/// exceptions. Written next to the snapshot by the list updater.
pub fn generic_selectors<'a>(lists: impl IntoIterator<Item = &'a str>) -> String {
    let mut hide = std::collections::BTreeSet::new();
    let mut except = HashSet::new();
    let simple = |selector: &str| {
        selector.len() > 1
            && matches!(selector.as_bytes()[0], b'.' | b'#')
            && selector[1..]
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    };
    for list in lists {
        for line in list.lines() {
            if let Some(selector) = line.strip_prefix("##") {
                if simple(selector) {
                    hide.insert(selector.to_string());
                }
            } else if let Some(selector) = line.strip_prefix("#@#") {
                if simple(selector) {
                    except.insert(selector.to_string());
                }
            }
        }
    }
    hide.into_iter()
        .filter(|selector| !except.contains(selector))
        .collect::<Vec<_>>()
        .join("\n")
}

impl AdBlocker {
    /// Loads the engine compiled into the binary, with the scriptlet and
    /// redirect resources. Fast; still best done off the UI thread.
    pub fn bundled() -> Self {
        let mut engine = Engine::default();
        // A snapshot from another build of the engine is ignored rather than
        // trusted: the browser then runs without lists, never with garbage.
        let _ = engine.deserialize(ENGINE_SNAPSHOT);
        if let Ok(resources) = serde_json::from_str::<Vec<Resource>>(RESOURCES) {
            engine.use_resources(resources);
        }
        Self::with_engine(engine)
    }

    /// Builds an engine from filter list texts in Adblock Plus syntax.
    pub fn from_lists<'a>(lists: impl IntoIterator<Item = &'a str>) -> Self {
        Self::with_engine(build_engine(lists))
    }

    fn with_engine(engine: Engine) -> Self {
        Self {
            engine,
            allowed_sites: RwLock::new(HashSet::new()),
            blocked: AtomicU64::new(0),
        }
    }

    /// Whether a request should be cancelled.
    ///
    /// `kind` is the resource kind in the vocabulary of filter lists
    /// (`script`, `image`, `sub_frame`, `xhr`, ...). Top-level navigations,
    /// non-web schemes and pages on the exception list are never blocked.
    pub fn should_block(&self, url: &str, page_url: &str, kind: &str) -> bool {
        if matches!(kind, "main_frame" | "document") {
            return false;
        }
        if self.site_allowed(page_url) {
            return false;
        }
        let Ok(request) = Request::new(url, page_url, kind, "GET") else {
            return false;
        };
        if !request.is_supported {
            return false;
        }
        let block = self.engine.check_network_request(&request).should_block();
        if block {
            self.blocked.fetch_add(1, Ordering::Relaxed);
        }
        block
    }

    /// What to hide and which scriptlets to run on the page at `url`.
    /// `top_level` pages also get the generic element hiding of the lists.
    pub fn cosmetic(&self, url: &str, top_level: bool) -> Cosmetic {
        if self.site_allowed(url) {
            return Cosmetic::default();
        }
        let found = self.engine.url_cosmetic_resources(url);
        let mut css = String::new();
        let mut selectors: Vec<&String> = found.hide_selectors.iter().collect();
        selectors.sort();
        for selector in selectors {
            // One rule each: a single bad selector would void a shared rule.
            css.push_str(selector);
            css.push_str("{display:none!important}\n");
        }
        if top_level && !found.generichide {
            // Plain class and id selectors are always valid, so they share
            // rules (a hundred at a time keeps the style sheet small).
            let mut lines = GENERIC_SELECTORS.lines().peekable();
            while lines.peek().is_some() {
                let group: Vec<&str> = lines.by_ref().take(100).collect();
                css.push_str(&group.join(","));
                css.push_str("{display:none!important}");
            }
        }
        Cosmetic {
            css,
            script: found.injected_script,
        }
    }

    /// Replaces the exception list. A listed host also covers its subdomains.
    pub fn set_allowed_sites<I, S>(&self, hosts: I)
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let hosts = hosts
            .into_iter()
            .map(|host| {
                host.as_ref()
                    .trim()
                    .trim_start_matches('.')
                    .to_ascii_lowercase()
            })
            .filter(|host| !host.is_empty())
            .collect();
        if let Ok(mut allowed) = self.allowed_sites.write() {
            *allowed = hosts;
        }
    }

    /// Requests cancelled since this engine was built.
    pub fn blocked_count(&self) -> u64 {
        self.blocked.load(Ordering::Relaxed)
    }

    fn site_allowed(&self, page_url: &str) -> bool {
        let Ok(allowed) = self.allowed_sites.read() else {
            return false;
        };
        if allowed.is_empty() {
            return false;
        }
        let Some(page) = parse_url(page_url) else {
            return false;
        };
        let host = page.hostname().to_ascii_lowercase();
        let mut candidate = host.as_str();
        loop {
            if allowed.contains(candidate) {
                return true;
            }
            match candidate.split_once('.') {
                Some((_, rest)) if rest.contains('.') => candidate = rest,
                _ => return false,
            }
        }
    }
}

/// The `! Version:` stamp of each bundled list, as `(title, version)`.
pub fn bundled_list_versions() -> Vec<(&'static str, String)> {
    VERSIONS
        .lines()
        .filter_map(|line| line.split_once('\t'))
        .map(|(title, version)| (title, version.trim().to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const RULES: &str =
        "||ads.example.net^\n||tracker.example.org^$third-party\n@@||ads.example.net/allowed.js\n";

    fn small() -> AdBlocker {
        AdBlocker::from_lists([RULES])
    }

    #[test]
    fn blocks_a_listed_host_and_counts_it() {
        let blocker = small();
        assert!(blocker.should_block(
            "https://ads.example.net/banner.png",
            "https://news.example.com/",
            "image"
        ));
        assert_eq!(blocker.blocked_count(), 1);
    }

    #[test]
    fn leaves_unlisted_requests_alone() {
        let blocker = small();
        assert!(!blocker.should_block(
            "https://news.example.com/app.js",
            "https://news.example.com/",
            "script"
        ));
        assert_eq!(blocker.blocked_count(), 0);
    }

    #[test]
    fn exception_rules_win() {
        let blocker = small();
        assert!(!blocker.should_block(
            "https://ads.example.net/allowed.js",
            "https://news.example.com/",
            "script"
        ));
    }

    #[test]
    fn third_party_option_is_respected() {
        let blocker = small();
        assert!(blocker.should_block(
            "https://tracker.example.org/t.js",
            "https://news.example.com/",
            "script"
        ));
        assert!(!blocker.should_block(
            "https://tracker.example.org/t.js",
            "https://tracker.example.org/",
            "script"
        ));
    }

    #[test]
    fn never_blocks_navigations() {
        let blocker = small();
        assert!(!blocker.should_block("https://ads.example.net/", "", "main_frame"));
        assert!(!blocker.should_block("https://ads.example.net/", "", "document"));
    }

    #[test]
    fn ignores_non_web_schemes_and_bad_urls() {
        let blocker = small();
        assert!(!blocker.should_block("agentgo://shell/chrome.html", "agentgo://shell/", "script"));
        assert!(!blocker.should_block("data:text/plain,hi", "https://news.example.com/", "image"));
        assert!(!blocker.should_block("not a url", "https://news.example.com/", "image"));
    }

    #[test]
    fn exception_list_covers_subdomains() {
        let blocker = small();
        blocker.set_allowed_sites(["Example.com"]);
        assert!(!blocker.should_block(
            "https://ads.example.net/banner.png",
            "https://news.example.com/",
            "image"
        ));
        assert!(!blocker.should_block(
            "https://ads.example.net/banner.png",
            "https://example.com/",
            "image"
        ));
        assert!(blocker.should_block(
            "https://ads.example.net/banner.png",
            "https://other.org/",
            "image"
        ));
        blocker.set_allowed_sites(Vec::<String>::new());
        assert!(blocker.should_block(
            "https://ads.example.net/banner.png",
            "https://news.example.com/",
            "image"
        ));
    }

    #[test]
    fn exception_list_does_not_match_lookalike_hosts() {
        let blocker = small();
        blocker.set_allowed_sites(["example.com"]);
        assert!(blocker.should_block(
            "https://ads.example.net/banner.png",
            "https://notexample.com/",
            "image"
        ));
    }

    #[test]
    fn bundled_lists_carry_versions() {
        let versions = bundled_list_versions();
        assert!(versions.len() >= 10, "{versions:?}");
        assert!(versions.iter().any(|(title, _)| title.contains("EasyList")));
    }

    #[test]
    fn cosmetic_rules_hide_elements_and_skip_allowed_sites() {
        let blocker = AdBlocker::from_lists([
            "##.sponsored-box\nexample.com##.promo\n##+js(set-constant, adsEnabled, false)\n",
        ]);
        let page = blocker.cosmetic("https://example.com/story", false);
        assert!(
            page.css.contains(".promo{display:none!important}"),
            "{page:?}"
        );
        let other = blocker.cosmetic("https://other.org/", false);
        assert!(!other.css.contains(".promo"));
        blocker.set_allowed_sites(["example.com"]);
        assert!(blocker
            .cosmetic("https://example.com/story", true)
            .is_empty());
    }

    #[test]
    fn generic_selectors_keep_plain_class_and_id_rules_only() {
        let text =
            "##.ad-banner\n##div > .nested\n###top-ad\nsite.com##.local\n##.kept\n#@#.kept\n";
        let out = generic_selectors([text]);
        let mut lines: Vec<&str> = out.lines().collect();
        lines.sort_unstable();
        assert_eq!(lines, ["#top-ad", ".ad-banner"]);
    }

    #[test]
    fn bundled_lists_block_known_ad_and_tracker_hosts_only() {
        let blocker = AdBlocker::bundled();
        let page = "https://news.example.com/article";
        for url in [
            "https://pagead2.googlesyndication.com/pagead/js/adsbygoogle.js",
            "https://www.google-analytics.com/analytics.js",
            "https://ads.pubmatic.com/AdServer/js/showad.js",
            "https://connect.facebook.net/en_US/fbevents.js",
            "https://static.hotjar.com/c/hotjar-1.js",
            "https://ads.yahoo.com/pixel",
            "https://an.facebook.com/tr",
            "https://mc.yandex.ru/metrika/tag.js",
            "https://analytics.tiktok.com/i18n/pixel/events.js",
            "https://bat.bing.com/bat.js",
            "https://ad.doubleclick.net/ddm/trackclk/",
            "https://sb.scorecardresearch.com/beacon.js",
        ] {
            assert!(blocker.should_block(url, page, "script"), "{url}");
        }
        assert!(!blocker.should_block("https://news.example.com/static/app.js", page, "script"));
        assert!(!blocker.should_block(
            "https://en.wikipedia.org/wiki/Rust_(programming_language)",
            page,
            "xhr"
        ));
        assert!(!blocker.should_block(
            "https://github.githubassets.com/assets/app.js",
            "https://github.com/",
            "script"
        ));
    }
}
