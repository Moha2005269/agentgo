//! YouTube ad removal at the network layer.
//!
//! YouTube sends its ads as data inside the player response (`adPlacements`,
//! `playerAds`, `adSlots`) and its feed ads as renderer objects. Removing
//! them from the response, before the page's scripts read it, leaves nothing
//! for the page to play and nothing in the page's own objects that was
//! changed. The rules are data (`youtube-rules.json`) so they can be updated
//! without rebuilding the browser.

use serde_json::Value;

const BUNDLED_RULES: &str = include_str!("../data/youtube-rules.json");
const BUNDLED_MODULE: &str = include_str!("../data/youtube-module.js");

/// What kind of body a response is, which decides how it is rewritten.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A JSON API response (`/youtubei/v1/player` and friends).
    Json,
    /// A page whose HTML carries `ytInitialPlayerResponse` / `ytInitialData`.
    Html,
}

#[derive(Clone, Debug)]
pub struct Rules {
    hosts: Vec<String>,
    json: Vec<String>,
    html: Vec<String>,
    html_exact: Vec<String>,
    prune_keys: Vec<String>,
    drop_item_keys: Vec<String>,
    html_markers: Vec<String>,
}

fn strings(value: &Value, key: &str) -> Vec<String> {
    value[key]
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

impl Rules {
    pub fn bundled() -> Self {
        Self::from_json(BUNDLED_RULES).expect("bundled YouTube rules are valid")
    }

    /// Parses a rules document; `None` if it is not one.
    pub fn from_json(text: &str) -> Option<Self> {
        let value: Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()?;
        let rules = Self {
            hosts: strings(&value, "hosts"),
            json: strings(&value, "json"),
            html: strings(&value, "html"),
            html_exact: strings(&value, "htmlExact"),
            prune_keys: strings(&value, "pruneKeys"),
            drop_item_keys: strings(&value, "dropItemKeys"),
            html_markers: strings(&value, "htmlMarkers"),
        };
        (!rules.hosts.is_empty()).then_some(rules)
    }

    /// The module script that ships with the browser.
    pub fn bundled_module() -> &'static str {
        BUNDLED_MODULE
    }

    /// The hosts these rules cover.
    pub fn hosts(&self) -> &[String] {
        &self.hosts
    }

    /// Whether the URL is on one of the covered hosts.
    pub fn covers(&self, url: &str) -> bool {
        self.split(url).is_some()
    }

    fn split<'a>(&self, url: &'a str) -> Option<&'a str> {
        let rest = url
            .strip_prefix("https://")
            .or_else(|| url.strip_prefix("http://"))?;
        let (host, path_and_query) = rest.split_once('/').map_or((rest, ""), |(h, p)| (h, p));
        let host = host.split(':').next().unwrap_or(host);
        self.hosts
            .iter()
            .any(|h| host == h || host.ends_with(&format!(".{h}")))
            .then_some(path_and_query)
    }

    /// How a response for this URL should be rewritten, if at all.
    pub fn kind_of(&self, url: &str) -> Option<Kind> {
        let path_and_query = self.split(url)?;
        let path = format!("/{}", path_and_query.split(['?', '#']).next().unwrap_or(""));
        if self.json.iter().any(|p| path.starts_with(p.as_str())) {
            return Some(Kind::Json);
        }
        if self.html_exact.contains(&path) {
            return Some(Kind::Html);
        }
        self.html
            .iter()
            .filter(|p| !self.html_exact.contains(p))
            .any(|p| path.starts_with(p.as_str()))
            .then_some(Kind::Html)
    }

    /// Removes the ad data from a response body. `None` when nothing needed
    /// to change (or the body is not what was expected), so the original is
    /// passed through untouched.
    pub fn rewrite(&self, kind: Kind, body: &[u8]) -> Option<Vec<u8>> {
        match kind {
            Kind::Json => {
                let mut value: Value = serde_json::from_slice(body).ok()?;
                self.prune(&mut value)
                    .then(|| serde_json::to_vec(&value).ok())?
            }
            Kind::Html => self.rewrite_html(body),
        }
    }

    fn rewrite_html(&self, body: &[u8]) -> Option<Vec<u8>> {
        let mut out = Vec::with_capacity(body.len());
        let mut cursor = 0;
        let mut changed = false;
        while let Some((start, marker_len)) = self.next_marker(body, cursor) {
            // `ytInitialPlayerResponse = {` / `window["ytInitialData"] = {`
            let after = start + marker_len;
            let Some(brace) = body[after..]
                .iter()
                .take(16)
                .position(|b| *b == b'{')
                .map(|at| after + at)
            else {
                out.extend_from_slice(&body[cursor..after]);
                cursor = after;
                continue;
            };
            let between = &body[after..brace];
            if !between
                .iter()
                .all(|b| matches!(b, b' ' | b'=' | b'"' | b'\'' | b']' | b'\n' | b'\t'))
            {
                out.extend_from_slice(&body[cursor..after]);
                cursor = after;
                continue;
            }
            let mut stream =
                serde_json::Deserializer::from_slice(&body[brace..]).into_iter::<Value>();
            let parsed = stream.next();
            let end = brace + stream.byte_offset();
            let cleaned = match parsed {
                Some(Ok(mut value)) => self.prune(&mut value).then_some(value),
                _ => None,
            };
            match cleaned {
                Some(value) => {
                    let json = serde_json::to_string(&value).ok()?.replace("</", "<\\/");
                    out.extend_from_slice(&body[cursor..brace]);
                    out.extend_from_slice(json.as_bytes());
                    cursor = end;
                    changed = true;
                }
                None => {
                    out.extend_from_slice(&body[cursor..brace]);
                    cursor = brace;
                    // Step past this marker so the search moves on.
                    out.extend_from_slice(&body[cursor..cursor + 1]);
                    cursor += 1;
                }
            }
        }
        if !changed {
            return None;
        }
        out.extend_from_slice(&body[cursor..]);
        Some(out)
    }

    fn next_marker(&self, body: &[u8], from: usize) -> Option<(usize, usize)> {
        self.html_markers
            .iter()
            .filter_map(|marker| {
                find(&body[from..], marker.as_bytes()).map(|at| (from + at, marker.len()))
            })
            .min_by_key(|(at, _)| *at)
    }

    /// Prunes ad keys and ad items in place; true if anything was removed.
    pub fn prune(&self, value: &mut Value) -> bool {
        let mut changed = false;
        match value {
            Value::Object(map) => {
                for key in &self.prune_keys {
                    changed |= map.remove(key).is_some();
                }
                for child in map.values_mut() {
                    changed |= self.prune(child);
                }
            }
            Value::Array(items) => {
                let before = items.len();
                items.retain(|item| !self.is_ad_item(item, 0));
                changed |= items.len() != before;
                for child in items.iter_mut() {
                    changed |= self.prune(child);
                }
            }
            _ => {}
        }
        changed
    }

    fn is_ad_item(&self, value: &Value, depth: usize) -> bool {
        if depth > 3 {
            return false;
        }
        match value {
            Value::Object(map) => {
                self.drop_item_keys.iter().any(|key| map.contains_key(key))
                    || map.values().any(|child| self.is_ad_item(child, depth + 1))
            }
            _ => false,
        }
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_youtube_api_and_watch_pages_are_rewritten() {
        let rules = Rules::bundled();
        assert_eq!(
            rules.kind_of("https://www.youtube.com/youtubei/v1/player?key=x"),
            Some(Kind::Json)
        );
        assert_eq!(
            rules.kind_of("https://www.youtube.com/watch?v=abc"),
            Some(Kind::Html)
        );
        assert_eq!(rules.kind_of("https://www.youtube.com/"), Some(Kind::Html));
        assert_eq!(
            rules.kind_of("https://www.youtube.com/s/player/base.js"),
            None
        );
        assert_eq!(rules.kind_of("https://example.com/watch?v=1"), None);
        assert_eq!(rules.kind_of("https://notyoutube.com/watch"), None);
    }

    #[test]
    fn player_json_loses_its_ads() {
        let rules = Rules::bundled();
        let body = br#"{"playabilityStatus":{"status":"OK"},"adPlacements":[{"x":1}],"playerAds":[1],"adSlots":[2],"videoDetails":{"title":"t"}}"#;
        let out = rules.rewrite(Kind::Json, body).expect("changed");
        let value: Value = serde_json::from_slice(&out).unwrap();
        assert!(value.get("adPlacements").is_none());
        assert!(value.get("playerAds").is_none());
        assert!(value.get("adSlots").is_none());
        assert_eq!(value["videoDetails"]["title"], "t");
    }

    #[test]
    fn json_without_ads_passes_through_untouched() {
        let rules = Rules::bundled();
        assert!(rules
            .rewrite(Kind::Json, br#"{"videoDetails":{"title":"t"}}"#)
            .is_none());
        assert!(rules.rewrite(Kind::Json, b"not json").is_none());
    }

    #[test]
    fn feed_ads_are_dropped_but_videos_stay() {
        let rules = Rules::bundled();
        let mut value: Value = serde_json::from_str(
            r#"{"contents":[{"richItemRenderer":{"content":{"videoRenderer":{"id":"a"}}}},{"richItemRenderer":{"content":{"adSlotRenderer":{"x":1}}}},{"videoRenderer":{"id":"b"}}]}"#,
        )
        .unwrap();
        assert!(rules.prune(&mut value));
        assert_eq!(value["contents"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn inline_player_response_in_html_is_cleaned() {
        let rules = Rules::bundled();
        let html = br#"<html><script>var ytInitialPlayerResponse = {"adPlacements":[{"a":"</script>"}],"videoDetails":{"title":"t"}};var other=1;</script><p>x</p></html>"#;
        let out = rules.rewrite(Kind::Html, html).expect("changed");
        let text = String::from_utf8(out).unwrap();
        assert!(!text.contains("adPlacements"));
        assert!(text.contains(r#""title":"t""#));
        assert!(text.contains("var other=1;</script><p>x</p></html>"));
    }

    #[test]
    fn html_without_ads_is_left_alone() {
        let rules = Rules::bundled();
        let html = br#"<script>var ytInitialData = {"contents":[{"videoRenderer":{}}]};</script>"#;
        assert!(rules.rewrite(Kind::Html, html).is_none());
    }

    #[test]
    fn rules_can_be_replaced_by_a_newer_document() {
        let rules =
            Rules::from_json(r#"{"hosts":["example.com"],"json":["/p"],"pruneKeys":["ads"]}"#)
                .unwrap();
        let out = rules.rewrite(Kind::Json, br#"{"ads":1,"v":2}"#).unwrap();
        assert_eq!(out, br#"{"v":2}"#);
        assert!(Rules::from_json("{}").is_none());
    }
}
