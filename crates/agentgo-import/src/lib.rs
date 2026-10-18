//! Read-only import of profile data from Chrome-family browsers and Firefox.
//!
//! Password values are intentionally omitted from debug output. Browser profile
//! data may be locked while a browser is running; SQLite files are copied,
//! including their WAL files, before they are read.

mod firefox;
mod snss;

use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM};
use rusqlite::{Connection, OpenFlags};
use serde_json::Value;
use std::collections::HashSet;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const CHROME_EPOCH_MS: i64 = 11_644_473_600_000;
static TEMP_ID: AtomicU64 = AtomicU64::new(0);

/// A supported source browser.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Browser {
    /// Google Chrome.
    Chrome,
    /// Microsoft Edge.
    Edge,
    /// Brave.
    Brave,
    /// Mozilla Firefox.
    Firefox,
}

/// A browser profile available for import.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Profile {
    /// Source browser.
    pub browser: Browser,
    /// Profile directory name (for example, `Default`).
    pub id: String,
    /// Human-readable profile name.
    pub name: String,
    /// Profile directory.
    pub path: PathBuf,
}

/// Approximate item counts available without decrypting passwords.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    /// Number of bookmark URLs.
    pub bookmarks: usize,
    /// Number of history records.
    pub history: usize,
    /// Number of saved-login rows.
    pub logins: usize,
    /// Number of open tabs found.
    pub tabs: usize,
    /// Number of Web Store extensions installed in the profile.
    pub extensions: usize,
}

/// An extension installed in the other browser, by its Web Store id. Only
/// the id and name are read; the package itself is fetched fresh from the
/// Web Store and checked against its signature, never copied from the
/// profile.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Extension {
    /// 32-letter Chrome Web Store id.
    pub id: String,
    /// Name from the extension's manifest (localised when it can be).
    pub name: String,
}

/// One imported bookmark.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bookmark {
    /// Bookmark title.
    pub title: String,
    /// HTTP or HTTPS URL.
    pub url: String,
    /// Folder path, such as `Bookmarks bar/News`.
    pub folder: String,
    /// Creation time in Unix milliseconds.
    pub added_ms: i64,
}

/// One imported history item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryItem {
    /// HTTP or HTTPS URL.
    pub url: String,
    /// Page title, possibly empty.
    pub title: String,
    /// Most recent visit in Unix milliseconds.
    pub last_visit_ms: i64,
    /// Recorded visit count.
    pub visits: u32,
}

/// One saved login. Passwords are never exposed by its `Debug` implementation.
#[derive(Clone, PartialEq, Eq)]
pub struct Login {
    /// Origin URL from the browser export.
    pub origin: String,
    /// Saved username.
    pub username: String,
    /// Saved password.
    pub password: String,
}

impl fmt::Debug for Login {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Login")
            .field("origin", &self.origin)
            .field("username", &self.username)
            .field("password", &"[REDACTED]")
            .finish()
    }
}

/// Data read from one profile.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Imported {
    /// Imported bookmarks.
    pub bookmarks: Vec<Bookmark>,
    /// Imported history.
    pub history: Vec<HistoryItem>,
    /// Decrypted logins or exported CSV logins.
    pub logins: Vec<Login>,
    /// Best-effort list of open-tab URLs.
    pub tabs: Vec<String>,
    /// Web Store extensions installed in the profile.
    pub extensions: Vec<Extension>,
    /// User-facing explanations of unavailable or approximate data.
    pub notes: Vec<String>,
}

/// Selects which categories to import.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Options {
    /// Import bookmarks.
    pub bookmarks: bool,
    /// Import history.
    pub history: bool,
    /// Import saved logins.
    pub logins: bool,
    /// Import open tabs.
    pub tabs: bool,
    /// List the profile's Web Store extensions (the caller installs them).
    pub extensions: bool,
    /// Maximum number of most-recent history records.
    pub history_limit: usize,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            bookmarks: true,
            history: true,
            logins: true,
            tabs: true,
            extensions: true,
            history_limit: 20_000,
        }
    }
}

/// Built-in extensions a Chromium profile carries that are part of the
/// browser rather than chosen by the person.
const BUILT_IN_EXTENSIONS: &[&str] = &[
    "nmmhkkegccagdldgiimedpiccmgmieda", // Chrome Web Store Payments
    "pkedcjkdefgpdelpbcmbmeomcjbeemfm", // Chrome Media Router
    "mhjfbmdgcfjbbpaeojofohoefgiehjai", // Chrome PDF Viewer
    "neajdppkdcdipfabeoofebfddakdcjhd", // Google Network Speech
    "ahfgeienlihckogmohjhadlkjgocpleb", // Web Store
];

/// The Web Store extensions of a Chromium profile: ids from its
/// `Extensions` folder, names from each newest version's manifest.
pub fn chromium_extensions(profile_dir: &Path) -> Vec<Extension> {
    let Ok(entries) = fs::read_dir(profile_dir.join("Extensions")) else {
        return Vec::new();
    };
    // What the browser recorded about each one, where it says so: only
    // extensions from the Web Store count (not policy, unpacked or bundled).
    let prefs: Value = ["Secure Preferences", "Preferences"]
        .iter()
        .filter_map(|file| fs::read(profile_dir.join(file)).ok())
        .filter_map(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .find(|json| json.pointer("/extensions/settings").is_some())
        .unwrap_or(Value::Null);
    let mut found: Vec<Extension> = entries
        .flatten()
        .filter_map(|entry| {
            let id = entry.file_name().to_string_lossy().into_owned();
            if id.len() != 32 || !id.bytes().all(|b| (b'a'..=b'p').contains(&b)) {
                return None;
            }
            if BUILT_IN_EXTENSIONS.contains(&id.as_str()) {
                return None;
            }
            let recorded = prefs.pointer(&format!("/extensions/settings/{id}"));
            if recorded.is_some_and(|r| r["from_webstore"] == false || r["location"].as_u64().is_some_and(|l| l != 1)) {
                return None;
            }
            let version = fs::read_dir(entry.path())
                .ok()?
                .flatten()
                .filter(|v| v.path().join("manifest.json").is_file())
                .max_by_key(|v| v.file_name())?;
            let name = extension_name(&version.path()).unwrap_or_else(|| id.clone());
            Some(Extension { id, name })
        })
        .collect();
    found.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    found
}

fn extension_name(version_dir: &Path) -> Option<String> {
    let manifest: Value =
        serde_json::from_slice(&fs::read(version_dir.join("manifest.json")).ok()?).ok()?;
    let name = manifest["name"].as_str()?.to_string();
    let Some(key) = name.strip_prefix("__MSG_").and_then(|k| k.strip_suffix("__")) else {
        return Some(name);
    };
    let locale = manifest["default_locale"].as_str().unwrap_or("en");
    let messages: Value = serde_json::from_slice(
        &fs::read(version_dir.join("_locales").join(locale).join("messages.json")).ok()?,
    )
    .ok()?;
    messages
        .as_object()?
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .and_then(|(_, v)| v["message"].as_str().map(str::to_string))
}

/// Discover profiles in standard Windows locations, or under
/// `AGENTGO_IMPORT_ROOT` when set (useful for fixture/test profiles).
pub fn detect() -> Vec<Profile> {
    let mut profiles = Vec::new();
    for (browser, relative) in [
        (Browser::Chrome, Path::new("Google/Chrome/User Data")),
        (Browser::Edge, Path::new("Microsoft/Edge/User Data")),
        (
            Browser::Brave,
            Path::new("BraveSoftware/Brave-Browser/User Data"),
        ),
    ] {
        if let Some(root) = local_root() {
            discover_chromium(&mut profiles, browser, &root.join(relative));
        }
    }
    if let Some(root) = app_root() {
        let dir = root.join("Mozilla/Firefox/Profiles");
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() && path.join("places.sqlite").is_file() {
                    let id = entry.file_name().to_string_lossy().into_owned();
                    profiles.push(Profile {
                        browser: Browser::Firefox,
                        name: id.clone(),
                        id,
                        path,
                    });
                }
            }
        }
    }
    profiles
}

/// Return inexpensive counts. SQLite counts are best-effort and are zero when
/// a database cannot be opened; passwords are not decrypted.
pub fn count(profile: &Profile) -> Counts {
    if profile.browser == Browser::Firefox {
        let (bookmarks, history) = if let Ok(conn) =
            open_readonly(&profile.path.join("places.sqlite"))
        {
            (
                scalar_count(
                    &conn,
                    "SELECT COUNT(*) FROM moz_bookmarks b JOIN moz_places p ON p.id=b.fk WHERE b.type=1",
                ),
                scalar_count(&conn, "SELECT COUNT(*) FROM moz_places WHERE visit_count > 0"),
            )
        } else {
            (0, 0)
        };
        return Counts {
            bookmarks,
            history,
            logins: json_array_len(&profile.path.join("logins.json"), "logins"),
            tabs: firefox::session_tabs(&profile.path).len(),
            // Firefox add-ons are not Chrome Web Store extensions.
            extensions: 0,
        };
    }

    // A running browser keeps an exclusive lock on its live databases, so
    // counts read the same snapshot copies the import reads.
    let history = copied_database(&profile.path.join("History"))
        .ok()
        .flatten()
        .map(|db| scalar_count(&db, "SELECT COUNT(*) FROM urls"))
        .unwrap_or(0);
    let logins: usize = CHROMIUM_LOGIN_FILES
        .iter()
        .filter_map(|file| copied_database(&profile.path.join(file)).ok().flatten())
        .map(|db| {
            scalar_count(
                &db,
                "SELECT COUNT(*) FROM logins WHERE blacklisted_by_user = 0 AND username_value <> ''",
            )
        })
        .sum();
    let tabs = chromium_tabs(&profile.path).0;
    Counts {
        bookmarks: chromium_bookmarks(&profile.path.join("Bookmarks"))
            .map_or(0, |items| items.len()),
        history,
        logins,
        tabs: tabs.len(),
        extensions: chromium_extensions(&profile.path).len(),
    }
}

/// Import selected categories from a profile. Individual unavailable
/// categories are reported in `notes`; malformed profile data does not panic.
pub fn import(profile: &Profile, options: &Options) -> Result<Imported, String> {
    let mut imported = Imported::default();
    if profile.browser == Browser::Firefox {
        import_firefox(profile, options, &mut imported);
    } else {
        import_chromium(profile, options, &mut imported);
    }
    Ok(imported)
}

/// Parse Chrome, Edge, or Firefox password-manager CSV exports.
///
/// Recognized headers include `name,url,username,password` and
/// `url,username,password`. Invalid, blank, and non-HTTP(S) rows are omitted.
pub fn parse_password_csv(text: &str) -> Vec<Login> {
    let rows = parse_csv_rows(text);
    let Some(header) = rows.first() else {
        return Vec::new();
    };
    let headers: Vec<String> = header
        .iter()
        .map(|value| {
            value
                .trim()
                .trim_start_matches('\u{feff}')
                .to_ascii_lowercase()
        })
        .collect();
    let url_col = headers
        .iter()
        .position(|h| matches!(h.as_str(), "url" | "origin" | "origin_url"));
    let user_col = headers
        .iter()
        .position(|h| matches!(h.as_str(), "username" | "username_value" | "user"));
    let pass_col = headers
        .iter()
        .position(|h| matches!(h.as_str(), "password" | "password_value" | "pass"));
    let (Some(url_col), Some(user_col), Some(pass_col)) = (url_col, user_col, pass_col) else {
        return Vec::new();
    };
    rows.iter()
        .skip(1)
        .filter_map(|row| {
            let origin = row.get(url_col)?.trim();
            let username = row.get(user_col)?.trim();
            let password = row.get(pass_col)?;
            (valid_web_url(origin) && !username.is_empty() && !password.is_empty()).then(|| Login {
                origin: origin.to_owned(),
                username: username.to_owned(),
                password: password.to_owned(),
            })
        })
        .collect()
}

fn import_chromium(profile: &Profile, options: &Options, out: &mut Imported) {
    if options.extensions {
        out.extensions = chromium_extensions(&profile.path);
    }
    let bookmarks_file = profile.path.join("Bookmarks");
    if options.bookmarks && bookmarks_file.exists() {
        match fs::read(&bookmarks_file)
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        {
            Some(json) => {
                let mut seen = HashSet::new();
                for (root, label) in [
                    ("bookmark_bar", "Bookmarks bar"),
                    ("other", "Other bookmarks"),
                    ("synced", "Mobile bookmarks"),
                ] {
                    if let Some(node) = json.get("roots").and_then(|roots| roots.get(root)) {
                        collect_chrome_bookmarks(node, label, out, &mut seen);
                    }
                }
            }
            None => note_once(out, "Bookmarks could not be read."),
        }
    }

    if options.history {
        match copied_database(&profile.path.join("History")) {
            Ok(Some(conn)) => {
                if let Ok(mut stmt) = conn.prepare(
                    "SELECT url, title, last_visit_time, visit_count FROM urls ORDER BY last_visit_time DESC LIMIT ?1",
                ) {
                    if let Ok(rows) = stmt.query_map([options.history_limit.min(i64::MAX as usize) as i64], |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1).unwrap_or_default(),
                            r.get::<_, i64>(2)?,
                            r.get::<_, i64>(3).unwrap_or(0),
                        ))
                    }) {
                        for row in rows.flatten() {
                            if valid_web_url(&row.0) {
                                out.history.push(HistoryItem {
                                    url: row.0,
                                    title: row.1,
                                    last_visit_ms: row.2 / 1000 - CHROME_EPOCH_MS,
                                    visits: row.3.max(0).min(u32::MAX as i64) as u32,
                                });
                            }
                        }
                    }
                } else {
                    note_once(out, "Browser history database has an unsupported format.");
                }
            }
            Ok(None) => {}
            Err(_) => note_once(out, "Close Chrome and try again: browser history is locked or could not be copied."),
        }
    }

    if options.logins {
        import_chromium_logins(profile, out);
    }

    if options.tabs {
        let (tabs, approximate) = chromium_tabs(&profile.path);
        out.tabs = tabs;
        if !out.tabs.is_empty() {
            if approximate {
                note_once(out, "Open tabs were approximated by extracting printable HTTP(S) URLs from session files; tab boundaries and the active navigation may be incomplete.");
            }
        } else if profile.path.join("Sessions").exists() {
            note_once(
                out,
                "No open tabs were found in the browser's session files.",
            );
        }
    }
}

/// Open tabs of a Chromium profile, newest session file first. The flag is
/// true when the structured parser failed and URLs were scraped instead.
fn chromium_tabs(profile: &Path) -> (Vec<String>, bool) {
    const LIMIT: usize = 50;
    let files: Vec<Vec<u8>> = session_files(profile)
        .into_iter()
        .filter_map(|file| fs::read(file).ok())
        .collect();
    let mut urls: Vec<String> = Vec::new();
    for bytes in &files {
        for url in snss::tabs(bytes).unwrap_or_default() {
            if !urls.contains(&url) {
                urls.push(url);
            }
        }
        if urls.len() >= LIMIT {
            urls.truncate(LIMIT);
            return (urls, false);
        }
    }
    if !urls.is_empty() {
        return (urls, false);
    }
    for bytes in &files {
        for url in extract_printable_urls(bytes) {
            if !urls.contains(&url) {
                urls.push(url);
            }
        }
        if urls.len() >= LIMIT {
            break;
        }
    }
    urls.truncate(LIMIT);
    let approximate = !urls.is_empty();
    (urls, approximate)
}
fn import_chromium_logins(profile: &Profile, out: &mut Imported) {
    let key = fs::read(
        profile
            .path
            .parent()
            .unwrap_or(&profile.path)
            .join("Local State"),
    )
    .ok()
    .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
    .and_then(|json| {
        json.pointer("/os_crypt/encrypted_key")
            .and_then(Value::as_str)
            .map(str::to_owned)
    })
    .and_then(|value| decode_base64(&value))
    .and_then(|mut bytes| {
        if bytes.starts_with(b"DPAPI") {
            bytes.drain(..5);
            dpapi_unprotect(&bytes).ok()
        } else {
            None
        }
    });
    let mut app_bound = 0usize;
    let mut unreadable = 0usize;
    let mut seen = HashSet::new();
    // Chrome keeps passwords saved to the Google account in a second file.
    for file in CHROMIUM_LOGIN_FILES {
        let path = profile.path.join(file);
        let conn = match copied_database(&path) {
            Ok(Some(conn)) => conn,
            Ok(None) => continue,
            Err(_) => {
                note_once(
                    out,
                    "Close the other browser and try again: its saved-login database is locked.",
                );
                continue;
            }
        };
        let Ok(mut stmt) = conn.prepare(
            "SELECT origin_url, username_value, password_value, blacklisted_by_user FROM logins",
        ) else {
            note_once(out, "Saved-login database has an unsupported format.");
            continue;
        };
        read_chromium_logins(
            &mut stmt,
            key.as_deref(),
            &mut seen,
            &mut app_bound,
            &mut unreadable,
            out,
        );
    }
    if app_bound > 0 {
        note_once(
            out,
            &format!("{app_bound} passwords use Chrome's app-bound encryption and can't be read by another program; export them from chrome://password-manager/settings as CSV and import that file"),
        );
    }
    if unreadable > 0 {
        note_once(
            out,
            &format!("{unreadable} saved passwords could not be decrypted for this Windows user; export them from the browser password manager as CSV."),
        );
    }
}

/// Saved-login databases of a Chromium profile: on this device, and saved to
/// the browser account.
const CHROMIUM_LOGIN_FILES: [&str; 2] = ["Login Data", "Login Data For Account"];

fn read_chromium_logins(
    stmt: &mut rusqlite::Statement<'_>,
    key: Option<&[u8]>,
    seen: &mut HashSet<(String, String)>,
    app_bound: &mut usize,
    unreadable: &mut usize,
    out: &mut Imported,
) {
    if let Ok(rows) = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Vec<u8>>(2)?,
            row.get::<_, i64>(3).unwrap_or(0),
        ))
    }) {
        for row in rows.flatten() {
            let (origin, username, encrypted, blacklisted) = row;
            if blacklisted != 0 || username.trim().is_empty() || !valid_web_url(&origin) {
                continue;
            }
            if !seen.insert((origin.clone(), username.clone())) {
                continue;
            }
            if encrypted.starts_with(b"v20") {
                *app_bound += 1;
                continue;
            }
            let password = if encrypted.starts_with(b"v10") || encrypted.starts_with(b"v11") {
                key.and_then(|key| decrypt_v10_password(key, &encrypted[3..]).ok())
            } else {
                dpapi_unprotect(&encrypted)
                    .ok()
                    .and_then(|bytes| String::from_utf8(bytes).ok())
            };
            if let Some(password) = password {
                out.logins.push(Login {
                    origin,
                    username,
                    password,
                });
            } else {
                *unreadable += 1;
            }
        }
    }
}

fn import_firefox(profile: &Profile, options: &Options, out: &mut Imported) {
    if options.bookmarks || options.history {
        match copied_database(&profile.path.join("places.sqlite")) {
            Ok(Some(conn)) => {
                if options.bookmarks {
                    read_firefox_bookmarks(&conn, out);
                }
                if options.history {
                    let limit = options.history_limit.min(i64::MAX as usize) as i64;
                    if let Ok(mut stmt) = conn.prepare(
                        "SELECT url, COALESCE(title,''), last_visit_date, visit_count FROM moz_places WHERE visit_count > 0 ORDER BY last_visit_date DESC LIMIT ?1",
                    ) {
                        if let Ok(rows) = stmt.query_map([limit], |row| {
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, i64>(2)?,
                                row.get::<_, i64>(3)?,
                            ))
                        }) {
                            for row in rows.flatten() {
                                if valid_web_url(&row.0) {
                                    out.history.push(HistoryItem {
                                        url: row.0,
                                        title: row.1,
                                        last_visit_ms: row.2 / 1000,
                                        visits: row.3.max(0).min(u32::MAX as i64) as u32,
                                    });
                                }
                            }
                        }
                    }
                }
            }
            Ok(None) => {}
            Err(_) => note_once(
                out,
                "Close Firefox and try again: places database is locked or could not be copied.",
            ),
        }
    }
    if options.logins {
        match firefox::read_logins(&profile.path) {
            firefox::LoginsResult::Logins(logins) => out.logins.extend(logins),
            firefox::LoginsResult::NoLogins => {}
            firefox::LoginsResult::PrimaryPassword => note_once(
                out,
                "Firefox passwords are locked by a primary password; remove it temporarily or export them from about:logins as CSV.",
            ),
            firefox::LoginsResult::Unreadable => note_once(
                out,
                "Firefox passwords could not be decrypted; export them from about:logins as CSV.",
            ),
        }
    }
    if options.tabs {
        out.tabs = firefox::session_tabs(&profile.path);
        out.tabs.truncate(50);
    }
}
fn read_firefox_bookmarks(conn: &Connection, out: &mut Imported) {
    let mut folder_names = std::collections::HashMap::<i64, (String, i64)>::new();
    if let Ok(mut stmt) = conn.prepare("SELECT id, title, parent FROM moz_bookmarks WHERE type=2") {
        if let Ok(rows) = stmt.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1).unwrap_or_default(),
                row.get::<_, i64>(2)?,
            ))
        }) {
            for row in rows.flatten() {
                folder_names.insert(row.0, (row.1, row.2));
            }
        }
    }
    let mut roots = std::collections::HashMap::<i64, String>::new();
    if let Ok(mut stmt) = conn.prepare("SELECT id, root_name FROM moz_bookmarks_roots") {
        if let Ok(rows) = stmt.query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        }) {
            for row in rows.flatten() {
                roots.insert(row.0, row.1);
            }
        }
    }
    let mut seen = HashSet::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT b.title, p.url, b.dateAdded, b.parent FROM moz_bookmarks b JOIN moz_places p ON p.id=b.fk WHERE b.type=1",
    ) {
        if let Ok(rows) = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0).unwrap_or_default(),
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2).unwrap_or(0),
                row.get::<_, i64>(3)?,
            ))
        }) {
            for row in rows.flatten() {
                if !valid_web_url(&row.1) || !seen.insert((row.1.clone(), row.3)) {
                    continue;
                }
                let folder = firefox_folder_path(row.3, &folder_names, &roots);
                out.bookmarks.push(Bookmark {
                    title: row.0,
                    url: row.1,
                    folder,
                    added_ms: row.2 / 1000,
                });
            }
        }
    }
}

fn firefox_folder_path(
    parent: i64,
    folders: &std::collections::HashMap<i64, (String, i64)>,
    roots: &std::collections::HashMap<i64, String>,
) -> String {
    let mut names = Vec::new();
    let mut current = parent;
    let mut visited = HashSet::new();
    while visited.insert(current) {
        if let Some(root) = roots.get(&current) {
            names.push(
                match root.as_str() {
                    "toolbar" => "Bookmarks bar",
                    "menu" => "Bookmarks menu",
                    "unfiled" => "Other bookmarks",
                    _ => root,
                }
                .to_owned(),
            );
            break;
        }
        let Some((name, next)) = folders.get(&current) else {
            break;
        };
        let root_name = match name.as_str() {
            "Bookmarks Toolbar" | "Bookmarks bar" => Some("Bookmarks bar"),
            "Bookmarks Menu" | "Bookmarks menu" => Some("Bookmarks menu"),
            "Other Bookmarks" | "Unfiled Bookmarks" => Some("Other bookmarks"),
            _ => None,
        };
        if let Some(root_name) = root_name {
            names.push(root_name.to_owned());
            break;
        }
        if !name.is_empty() {
            names.push(name.clone());
        }
        current = *next;
    }
    names.reverse();
    names.join("/")
}

fn collect_chrome_bookmarks(
    node: &Value,
    folder: &str,
    out: &mut Imported,
    seen: &mut HashSet<(String, String)>,
) {
    let Some(children) = node.get("children").and_then(Value::as_array) else {
        return;
    };
    for child in children {
        let name = child.get("name").and_then(Value::as_str).unwrap_or("");
        if let Some(url) = child.get("url").and_then(Value::as_str) {
            if valid_web_url(url) && seen.insert((url.to_owned(), folder.to_owned())) {
                let raw = child
                    .get("date_added")
                    .and_then(Value::as_str)
                    .unwrap_or("0");
                let added_ms = raw.parse::<i64>().unwrap_or(0) / 1000 - CHROME_EPOCH_MS;
                out.bookmarks.push(Bookmark {
                    title: name.to_owned(),
                    url: url.to_owned(),
                    folder: folder.to_owned(),
                    added_ms,
                });
            }
        } else if child.get("type").and_then(Value::as_str) == Some("folder") {
            let child_folder = if folder.is_empty() {
                name.to_owned()
            } else {
                format!("{folder}/{name}")
            };
            collect_chrome_bookmarks(child, &child_folder, out, seen);
        }
    }
}

fn chromium_bookmarks(path: &Path) -> Option<Vec<Bookmark>> {
    let bytes = fs::read(path).ok()?;
    let json: Value = serde_json::from_slice(&bytes).ok()?;
    let mut out = Imported::default();
    let mut seen = HashSet::new();
    for (root, label) in [
        ("bookmark_bar", "Bookmarks bar"),
        ("other", "Other bookmarks"),
        ("synced", "Mobile bookmarks"),
    ] {
        if let Some(node) = json.get("roots").and_then(|v| v.get(root)) {
            collect_chrome_bookmarks(node, label, &mut out, &mut seen);
        }
    }
    Some(out.bookmarks)
}

fn discover_chromium(profiles: &mut Vec<Profile>, browser: Browser, root: &Path) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let id = entry.file_name().to_string_lossy().into_owned();
        if !path.is_dir()
            || !(path.join("Bookmarks").is_file() || path.join("History").is_file())
            || !(id == "Default" || id.starts_with("Profile "))
        {
            continue;
        }
        let name = fs::read(path.join("Preferences"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .and_then(|json| {
                json.pointer("/profile/name")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .or_else(|| {
                fs::read(root.join("Local State"))
                    .ok()
                    .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                    .and_then(|json| {
                        json.pointer(&format!("/profile/info_cache/{id}/name"))
                            .and_then(Value::as_str)
                            .map(str::to_owned)
                    })
            })
            .unwrap_or_else(|| id.clone());
        profiles.push(Profile {
            browser,
            id,
            name,
            path,
        });
    }
}

fn local_root() -> Option<PathBuf> {
    if let Some(root) = std::env::var_os("AGENTGO_IMPORT_ROOT") {
        return Some(PathBuf::from(root));
    }
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
}

fn app_root() -> Option<PathBuf> {
    if let Some(root) = std::env::var_os("AGENTGO_IMPORT_ROOT") {
        return Some(PathBuf::from(root));
    }
    std::env::var_os("APPDATA").map(PathBuf::from)
}

fn open_readonly(path: &Path) -> rusqlite::Result<Connection> {
    Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
}

fn scalar_count(conn: &Connection, sql: &str) -> usize {
    conn.query_row(sql, [], |row| row.get::<_, i64>(0))
        .map(|value| value.max(0) as usize)
        .unwrap_or(0)
}

fn json_array_len(path: &Path, key: &str) -> usize {
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|json| json.get(key).and_then(Value::as_array).map(Vec::len))
        .unwrap_or(0)
}

struct TempCopy(PathBuf);

impl Drop for TempCopy {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Snapshot {
    connection: Connection,
    _temp: TempCopy,
}

impl std::ops::Deref for Snapshot {
    type Target = Connection;

    fn deref(&self) -> &Self::Target {
        &self.connection
    }
}

fn copied_database(path: &Path) -> Result<Option<Snapshot>, String> {
    if !path.is_file() {
        return Ok(None);
    }
    let id = TEMP_ID.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("agentgo-import-{}-{id}", std::process::id()));
    fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let guard = TempCopy(dir.clone());
    let dest = dir.join("snapshot.sqlite");
    fs::copy(path, &dest).map_err(|e| e.to_string())?;
    let wal = PathBuf::from(format!("{}-wal", path.display()));
    if wal.exists() {
        fs::copy(wal, PathBuf::from(format!("{}-wal", dest.display())))
            .map_err(|e| e.to_string())?;
    }
    let shm = PathBuf::from(format!("{}-shm", path.display()));
    if shm.exists() {
        fs::copy(shm, PathBuf::from(format!("{}-shm", dest.display())))
            .map_err(|e| e.to_string())?;
    }
    let conn = open_readonly(&dest).map_err(|e| e.to_string())?;
    Ok(Some(Snapshot {
        connection: conn,
        _temp: guard,
    }))
}

fn session_files(profile: &Path) -> Vec<PathBuf> {
    let dir = profile.join("Sessions");
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<_> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("Tabs_") || name.starts_with("Session_"))
        })
        .collect();
    files.sort_by_key(|path| std::cmp::Reverse(path.metadata().and_then(|m| m.modified()).ok()));
    files
}

fn extract_printable_urls(bytes: &[u8]) -> Vec<String> {
    let mut output = Vec::new();
    let mut text = String::new();
    for &byte in bytes {
        if byte.is_ascii_graphic() || byte == b' ' {
            text.push(byte as char);
        } else {
            extract_urls_from_text(&text, &mut output);
            text.clear();
        }
    }
    extract_urls_from_text(&text, &mut output);
    output
}

fn extract_urls_from_text(text: &str, output: &mut Vec<String>) {
    let mut remaining = text;
    while let Some(index) = remaining
        .find("http://")
        .or_else(|| remaining.find("https://"))
    {
        let candidate = &remaining[index..];
        let end = candidate
            .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | '\\' | '\0'))
            .unwrap_or(candidate.len());
        let url = candidate[..end].trim_end_matches(['.', ',', ';', ')', ']']);
        if valid_web_url(url) {
            output.push(url.to_owned());
        }
        remaining = &candidate[end.max(1)..];
    }
}

fn valid_web_url(url: &str) -> bool {
    if url.trim() != url || url.chars().any(char::is_whitespace) {
        return false;
    }
    let Some((scheme, rest)) = url.split_once("://") else {
        return false;
    };
    if !matches!(scheme.to_ascii_lowercase().as_str(), "http" | "https") {
        return false;
    }
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    !host.is_empty() && !host.chars().any(char::is_control)
}

fn note_once(out: &mut Imported, note: &str) {
    if !out.notes.iter().any(|existing| existing == note) {
        out.notes.push(note.to_owned());
    }
}

fn decode_base64(input: &str) -> Option<Vec<u8>> {
    let mut output = Vec::new();
    let mut accumulator = 0u32;
    let mut bits = 0u8;
    for byte in input
        .bytes()
        .filter(|b| !b.is_ascii_whitespace() && *b != b'=')
    {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        };
        accumulator = (accumulator << 6) | value as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            output.push((accumulator >> bits) as u8);
        }
    }
    Some(output)
}

fn decrypt_v10_password(key: &[u8], nonce_ciphertext_tag: &[u8]) -> Result<String, String> {
    if nonce_ciphertext_tag.len() < 12 + 16 {
        return Err("encrypted password is too short".into());
    }
    let unbound = UnboundKey::new(&AES_256_GCM, key).map_err(|_| "invalid AES-256 key")?;
    let key = LessSafeKey::new(unbound);
    let nonce = Nonce::try_assume_unique_for_key(&nonce_ciphertext_tag[..12])
        .map_err(|_| "invalid AES-GCM nonce")?;
    let mut plaintext = nonce_ciphertext_tag[12..].to_vec();
    let clear = key
        .open_in_place(nonce, Aad::empty(), &mut plaintext)
        .map_err(|_| "password authentication failed")?;
    String::from_utf8(clear.to_vec()).map_err(|_| "password is not UTF-8".into())
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn dpapi_unprotect(input: &[u8]) -> Result<Vec<u8>, String> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };
    let data = CRYPT_INTEGER_BLOB {
        cbData: input.len() as u32,
        pbData: input.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    // SAFETY: data references the input slice and Windows allocates output.
    let success = unsafe {
        CryptUnprotectData(
            &data,
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    if success == 0 || output.pbData.is_null() {
        return Err(std::io::Error::last_os_error().to_string());
    }
    // SAFETY: on success output is a Windows-owned buffer of cbData bytes.
    let result =
        unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) }.to_vec();
    // SAFETY: output.pbData was allocated by Windows for this call.
    unsafe { LocalFree(output.pbData.cast()) };
    Ok(result)
}

#[cfg(not(windows))]
fn dpapi_unprotect(_input: &[u8]) -> Result<Vec<u8>, String> {
    Err("DPAPI is available only on Windows".into())
}

fn parse_csv_rows(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut chars = text.chars().peekable();
    let mut quoted = false;
    while let Some(ch) = chars.next() {
        match ch {
            '"' if quoted && chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            ',' if !quoted => row.push(std::mem::take(&mut field)),
            '\n' if !quoted => {
                row.push(std::mem::take(&mut field));
                if row.iter().any(|cell| !cell.is_empty()) {
                    rows.push(std::mem::take(&mut row));
                } else {
                    row.clear();
                }
            }
            '\r' if !quoted => {}
            _ => field.push(ch),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_web_store_extensions_with_their_names() {
        let dir = std::env::temp_dir().join(format!("agentgo-import-ext-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let put = |rel: &str, body: &str| {
            let path = dir.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, body).unwrap();
        };
        // Dark Reader, two versions (the newer one names it), localised.
        put("Extensions/eimadpbcbfnmbkopoojfekhnkhdbieeh/4.9.1_0/manifest.json", r#"{"name":"Old"}"#);
        put("Extensions/eimadpbcbfnmbkopoojfekhnkhdbieeh/4.9.9_0/manifest.json", r#"{"name":"__MSG_extension_name__","default_locale":"en"}"#);
        put("Extensions/eimadpbcbfnmbkopoojfekhnkhdbieeh/4.9.9_0/_locales/en/messages.json", r#"{"extension_name":{"message":"Dark Reader"}}"#);
        // A browser built-in, an unpacked one and a stray folder are not listed.
        put("Extensions/nmmhkkegccagdldgiimedpiccmgmieda/1.0_0/manifest.json", r#"{"name":"Payments"}"#);
        put("Extensions/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/1.0_0/manifest.json", r#"{"name":"Unpacked"}"#);
        put("Extensions/Temp/manifest.json", "{}");
        put("Secure Preferences", r#"{"extensions":{"settings":{"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa":{"location":4,"from_webstore":false},"eimadpbcbfnmbkopoojfekhnkhdbieeh":{"location":1,"from_webstore":true}}}}"#);
        assert_eq!(
            chromium_extensions(&dir),
            vec![Extension { id: "eimadpbcbfnmbkopoojfekhnkhdbieeh".into(), name: "Dark Reader".into() }]
        );
        assert!(chromium_extensions(&dir.join("missing")).is_empty());
        let _ = fs::remove_dir_all(&dir);
    }
    use ring::aead::{Aad, Nonce, UnboundKey, AES_256_GCM};
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TestDir(PathBuf);
    impl TestDir {
        fn new() -> Self {
            let id = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!("agentgo-import-test-{id}"));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn imports_chrome_bookmarks_and_history_from_fixtures() {
        let tmp = TestDir::new();
        fs::write(
            tmp.0.join("Bookmarks"),
            br#"{"roots":{"bookmark_bar":{"children":[{"type":"url","name":"Example","url":"https://example.com","date_added":"13222310400000000"},{"type":"url","name":"Skip","url":"javascript:alert(1)"}]}}}"#,
        ).unwrap();
        let conn = Connection::open(tmp.0.join("History")).unwrap();
        conn.execute_batch("CREATE TABLE urls(url TEXT,title TEXT,last_visit_time INTEGER,visit_count INTEGER); INSERT INTO urls VALUES('https://example.com','Example',13222310400000000,3),('chrome://settings','Settings',1,1);").unwrap();
        drop(conn);
        let profile = Profile {
            browser: Browser::Chrome,
            id: "Default".into(),
            name: "Test".into(),
            path: tmp.0.clone(),
        };
        let options = Options {
            logins: false,
            tabs: false,
            ..Options::default()
        };
        let imported = import(&profile, &options).unwrap();
        assert_eq!(imported.bookmarks.len(), 1);
        assert_eq!(imported.bookmarks[0].folder, "Bookmarks bar");
        assert_eq!(imported.history.len(), 1);
        assert_eq!(imported.history[0].visits, 3);
    }

    #[test]
    fn imports_firefox_places_bookmarks_and_history() {
        let tmp = TestDir::new();
        let conn = Connection::open(tmp.0.join("places.sqlite")).unwrap();
        conn.execute_batch(
            "CREATE TABLE moz_places(id INTEGER PRIMARY KEY,url TEXT,title TEXT,last_visit_date INTEGER,visit_count INTEGER);
             CREATE TABLE moz_bookmarks(id INTEGER PRIMARY KEY,fk INTEGER,type INTEGER,parent INTEGER,title TEXT,dateAdded INTEGER);
             CREATE TABLE moz_bookmarks_roots(root_name TEXT,id INTEGER);
             INSERT INTO moz_places VALUES(1,'https://mozilla.org','Mozilla',1700000000000000,4);
             INSERT INTO moz_bookmarks_roots VALUES('toolbar',2);
             INSERT INTO moz_bookmarks VALUES(3,1,1,2,'Mozilla',1600000000000000);",
        ).unwrap();
        drop(conn);
        let profile = Profile {
            browser: Browser::Firefox,
            id: "abc.default".into(),
            name: "Firefox".into(),
            path: tmp.0.clone(),
        };
        let options = Options {
            logins: false,
            tabs: false,
            ..Options::default()
        };
        let imported = import(&profile, &options).unwrap();
        assert_eq!(imported.bookmarks.len(), 1);
        assert_eq!(imported.bookmarks[0].folder, "Bookmarks bar");
        assert_eq!(imported.history[0].last_visit_ms, 1_700_000_000_000);
    }

    #[test]
    fn decrypts_aes_gcm_v10_payload_and_redacts_debug() {
        let key = [42u8; 32];
        let nonce_bytes = [7u8; 12];
        let unbound = UnboundKey::new(&AES_256_GCM, &key).unwrap();
        let sealing = LessSafeKey::new(unbound);
        let nonce = Nonce::assume_unique_for_key(nonce_bytes);
        let mut payload = b"secret".to_vec();
        sealing
            .seal_in_place_append_tag(nonce, Aad::empty(), &mut payload)
            .unwrap();
        let mut data = nonce_bytes.to_vec();
        data.extend(payload);
        assert_eq!(decrypt_v10_password(&key, &data).unwrap(), "secret");
        let login = Login {
            origin: "https://example.com".into(),
            username: "u".into(),
            password: "very-secret".into(),
        };
        assert!(!format!("{login:?}").contains("very-secret"));
    }

    #[test]
    fn parses_quoted_chrome_and_firefox_csv() {
        let rows = parse_password_csv("name,url,username,password\r\nExample,https://example.com,\"user,one\",\"p\"\"ass\"\r\n");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].username, "user,one");
        assert_eq!(rows[0].password, "p\"ass");
        let firefox = parse_password_csv(
            "url,username,password,hostname\nhttps://mozilla.org,me,pw,mozilla.org\n",
        );
        assert_eq!(firefox.len(), 1);
        assert_eq!(firefox[0].origin, "https://mozilla.org");
    }

    #[cfg(windows)]
    #[test]
    #[allow(unsafe_code)]
    fn dpapi_roundtrip_recovers_chrome_style_key() {
        use windows_sys::Win32::Foundation::LocalFree;
        use windows_sys::Win32::Security::Cryptography::{
            CryptProtectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
        };
        let secret = [0x42u8; 32];
        let data = CRYPT_INTEGER_BLOB {
            cbData: secret.len() as u32,
            pbData: secret.as_ptr() as *mut u8,
        };
        let mut out = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };
        // SAFETY: data points at a live slice; Windows allocates `out`.
        let ok = unsafe {
            CryptProtectData(
                &data,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
        };
        assert_ne!(ok, 0);
        // SAFETY: on success `out` is a Windows-owned buffer of cbData bytes.
        let blob = unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize) }.to_vec();
        // SAFETY: allocated by CryptProtectData above.
        unsafe { LocalFree(out.pbData.cast()) };
        assert_eq!(dpapi_unprotect(&blob).unwrap(), secret);
    }
}
