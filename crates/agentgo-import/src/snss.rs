//! Parser for Chromium "SNSS" session files (`Sessions/Session_*`,
//! `Sessions/Tabs_*`), recovering each live tab's selected URL.

use super::valid_web_url;
use std::collections::{BTreeMap, HashMap};

const CMD_UPDATE_TAB_NAVIGATION: u8 = 6;
const CMD_SET_SELECTED_NAVIGATION_INDEX: u8 = 7;
const CMD_TAB_CLOSED: u8 = 16;
const CMD_WINDOW_CLOSED: u8 = 17;
const CMD_SET_TAB_WINDOW: u8 = 0;
const SNSS_VERSION_PLAIN: [i32; 2] = [1, 3];

#[derive(Default)]
struct Tab {
    navigations: BTreeMap<i32, String>,
    selected: Option<i32>,
    window: Option<i32>,
}

/// Returns `None` when the file is not a readable plain SNSS file.
pub(crate) fn tabs(bytes: &[u8]) -> Option<Vec<String>> {
    if bytes.get(..4)? != b"SNSS" {
        return None;
    }
    let version = i32::from_le_bytes(bytes.get(4..8)?.try_into().ok()?);
    if !SNSS_VERSION_PLAIN.contains(&version) {
        return None;
    }
    let mut tabs: HashMap<i32, Tab> = HashMap::new();
    let mut order: Vec<i32> = Vec::new();
    let mut closed_windows: Vec<i32> = Vec::new();
    let mut pos = 8usize;
    while pos + 2 <= bytes.len() {
        let size = u16::from_le_bytes([bytes[pos], bytes[pos + 1]]) as usize;
        pos += 2;
        if size == 0 || pos + size > bytes.len() {
            break;
        }
        let id = bytes[pos];
        let payload = &bytes[pos + 1..pos + size];
        pos += size;
        match id {
            CMD_SET_TAB_WINDOW => {
                if let (Some(window), Some(tab)) = (int(payload, 0), int(payload, 4)) {
                    entry(&mut tabs, &mut order, tab).window = Some(window);
                }
            }
            CMD_UPDATE_TAB_NAVIGATION => {
                if let Some((tab, index, url)) = navigation(payload) {
                    entry(&mut tabs, &mut order, tab)
                        .navigations
                        .insert(index, url);
                }
            }
            CMD_SET_SELECTED_NAVIGATION_INDEX => {
                if let (Some(tab), Some(index)) = (int(payload, 0), int(payload, 4)) {
                    entry(&mut tabs, &mut order, tab).selected = Some(index);
                }
            }
            CMD_TAB_CLOSED => {
                if let Some(tab) = int(payload, 0) {
                    tabs.remove(&tab);
                    order.retain(|id| *id != tab);
                }
            }
            CMD_WINDOW_CLOSED => {
                if let Some(window) = int(payload, 0) {
                    closed_windows.push(window);
                }
            }
            _ => {}
        }
    }
    let mut urls: Vec<String> = Vec::new();
    for id in order {
        let Some(tab) = tabs.get(&id) else { continue };
        if tab.window.is_some_and(|w| closed_windows.contains(&w)) {
            continue;
        }
        let url = tab
            .selected
            .and_then(|index| tab.navigations.get(&index))
            .or_else(|| tab.navigations.values().next_back());
        if let Some(url) = url.filter(|url| valid_web_url(url)) {
            if !urls.contains(url) {
                urls.push(url.clone());
            }
        }
    }
    Some(urls)
}

fn entry<'a>(tabs: &'a mut HashMap<i32, Tab>, order: &mut Vec<i32>, id: i32) -> &'a mut Tab {
    if !tabs.contains_key(&id) {
        order.push(id);
    }
    tabs.entry(id).or_default()
}

fn int(payload: &[u8], offset: usize) -> Option<i32> {
    Some(i32::from_le_bytes(
        payload.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

/// Payload is a Pickle: u32 size, i32 tab id, i32 navigation index, string url.
fn navigation(payload: &[u8]) -> Option<(i32, i32, String)> {
    let tab = int(payload, 4)?;
    let index = int(payload, 8)?;
    let len = usize::try_from(int(payload, 12)?).ok()?;
    let url = payload.get(16..16usize.checked_add(len)?)?;
    Some((tab, index, String::from_utf8(url.to_vec()).ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(id: u8, payload: &[u8]) -> Vec<u8> {
        let mut out = ((payload.len() + 1) as u16).to_le_bytes().to_vec();
        out.push(id);
        out.extend_from_slice(payload);
        out
    }

    fn nav(tab: i32, index: i32, url: &str) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(&tab.to_le_bytes());
        body.extend_from_slice(&index.to_le_bytes());
        body.extend_from_slice(&(url.len() as i32).to_le_bytes());
        body.extend_from_slice(url.as_bytes());
        while body.len() % 4 != 0 {
            body.push(0);
        }
        let mut payload = (body.len() as u32).to_le_bytes().to_vec();
        payload.extend(body);
        command(CMD_UPDATE_TAB_NAVIGATION, &payload)
    }

    fn pair(id: u8, a: i32, b: i32) -> Vec<u8> {
        let mut payload = a.to_le_bytes().to_vec();
        payload.extend_from_slice(&b.to_le_bytes());
        command(id, &payload)
    }

    #[test]
    fn picks_selected_navigation_and_drops_closed_tabs() {
        let mut file = b"SNSS".to_vec();
        file.extend_from_slice(&1i32.to_le_bytes());
        for part in [
            pair(CMD_SET_TAB_WINDOW, 1, 10),
            nav(10, 0, "https://a.test/first"),
            nav(10, 1, "https://a.test/second"),
            pair(CMD_SET_SELECTED_NAVIGATION_INDEX, 10, 0),
            pair(CMD_SET_TAB_WINDOW, 1, 11),
            nav(11, 0, "https://b.test/"),
            pair(CMD_SET_TAB_WINDOW, 1, 12),
            nav(12, 0, "https://closed.test/"),
            pair(CMD_TAB_CLOSED, 12, 0),
            pair(CMD_SET_TAB_WINDOW, 2, 13),
            nav(13, 0, "https://gone-window.test/"),
            pair(CMD_WINDOW_CLOSED, 2, 0),
            nav(14, 0, "chrome://settings"),
        ] {
            file.extend(part);
        }
        assert_eq!(
            tabs(&file).unwrap(),
            vec![
                "https://a.test/first".to_owned(),
                "https://b.test/".to_owned()
            ]
        );
    }

    #[test]
    fn rejects_other_files_and_survives_truncation() {
        assert!(tabs(b"nonsense").is_none());
        let mut file = b"SNSS".to_vec();
        file.extend_from_slice(&1i32.to_le_bytes());
        file.extend(nav(1, 0, "https://a.test/"));
        file.extend_from_slice(&[0xFF, 0x7F, 6, 1]);
        assert_eq!(tabs(&file).unwrap(), vec!["https://a.test/".to_owned()]);
    }
}
