//! AgentGo extensions: the content scripts of Chrome extensions.
//!
//! Most extensions are, at heart, content scripts: JavaScript (and CSS) that
//! runs on the pages their `matches` patterns name. AgentGo reads a Chrome
//! extension folder (manifest version 2 or 3), keeps its content scripts in
//! the profile and runs them on every matching page, each extension in its
//! own isolated world, once the document has loaded (Chrome's default
//! `document_idle`). The person installs, enables and removes extensions;
//! agents can list them and read their settings.
//!
//! Not provided: the `chrome.*` extension APIs (toolbar buttons, context
//! menus, background workers, native messaging).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use agentgo_protocol::ErrorCode;
use serde_json::{json, Value};

use super::sessions::{Fail, Reply};
use super::{cdp, state};

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Extension {
    id: String,
    name: String,
    version: String,
    source: String,
    enabled: bool,
    permissions: Vec<String>,
    scripts: Vec<Script>,
    #[serde(default)]
    storage: BTreeMap<String, Value>,
    /// Shown as a button in the toolbar.
    #[serde(default)]
    pinned: bool,
    /// The extension's own icon as a data URL (empty when it has none).
    #[serde(default)]
    icon: String,
    #[serde(default)]
    description: String,
    /// The toolbar pop-up page (`action.default_popup`), relative to the folder.
    #[serde(default)]
    popup: String,
    /// The options page, relative to the folder.
    #[serde(default)]
    options: String,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Script {
    matches: Vec<String>,
    #[serde(default)]
    exclude: Vec<String>,
    js: String,
    css: String,
}

fn file() -> PathBuf {
    state::state().options.cache_root.join("extensions.json")
}

fn load() -> Vec<Extension> {
    std::fs::read(file())
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn save(all: &[Extension]) -> Result<(), Fail> {
    std::fs::write(file(), serde_json::to_vec_pretty(all).unwrap_or_default())
        .map_err(|error| Fail::internal(error.to_string()))
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Reads a Chrome extension folder.
fn read_folder(folder: &Path) -> Result<Extension, Fail> {
    let bad = |what: String| Fail::params(format!("{}: {what}", folder.display()));
    let manifest: Value = serde_json::from_slice(
        &std::fs::read(folder.join("manifest.json")).map_err(|_| bad("no manifest.json".into()))?,
    )
    .map_err(|error| bad(format!("manifest.json is not valid JSON ({error})")))?;
    let name = manifest["name"]
        .as_str()
        .filter(|name| !name.starts_with("__MSG_"))
        .unwrap_or("Extension")
        .chars()
        .take(60)
        .collect::<String>();
    let read = |relative: &str| -> Result<String, Fail> {
        let path = folder.join(relative);
        // Files must stay inside the extension's folder.
        let inside = path
            .canonicalize()
            .ok()
            .zip(folder.canonicalize().ok())
            .is_some_and(|(file, root)| file.starts_with(root));
        if !inside {
            return Err(bad(format!("{relative} is outside the extension")));
        }
        std::fs::read_to_string(&path).map_err(|_| bad(format!("cannot read {relative}")))
    };
    let mut scripts = Vec::new();
    for entry in manifest["content_scripts"]
        .as_array()
        .cloned()
        .unwrap_or_default()
    {
        let mut js = String::new();
        for relative in strings(&entry["js"]) {
            js.push_str(&read(&relative)?);
            js.push_str("\n;\n");
        }
        let mut css = String::new();
        for relative in strings(&entry["css"]) {
            css.push_str(&read(&relative)?);
            css.push('\n');
        }
        scripts.push(Script {
            matches: strings(&entry["matches"]),
            exclude: strings(&entry["exclude_matches"]),
            js,
            css,
        });
    }
    let page = |value: &Value| {
        value
            .as_str()
            .map(|path| path.trim_start_matches('/').to_string())
            .filter(|path| !path.is_empty() && read(path).is_ok())
            .unwrap_or_default()
    };
    let popup = page(&manifest["action"]["default_popup"]);
    let popup = if popup.is_empty() {
        page(&manifest["browser_action"]["default_popup"])
    } else {
        popup
    };
    let options = page(&manifest["options_ui"]["page"]);
    let options = if options.is_empty() {
        page(&manifest["options_page"])
    } else {
        options
    };
    if scripts.is_empty() && popup.is_empty() {
        return Err(bad(
            "the extension has neither content scripts nor a toolbar pop-up, the parts AgentGo runs".into(),
        ));
    }
    let mut permissions = strings(&manifest["permissions"]);
    permissions.extend(strings(&manifest["host_permissions"]));
    let id: String = name
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    Ok(Extension {
        id: id.trim_matches('-').chars().take(40).collect(),
        name,
        version: manifest["version"].as_str().unwrap_or("0").to_string(),
        source: folder.display().to_string(),
        enabled: true,
        permissions,
        scripts,
        storage: BTreeMap::new(),
        pinned: true,
        icon: icon_of(folder, &manifest),
        description: manifest["description"]
            .as_str()
            .filter(|text| !text.starts_with("__MSG_"))
            .unwrap_or("")
            .chars()
            .take(200)
            .collect(),
        popup,
        options,
    })
}

/// The largest PNG icon (up to 128 px) the manifest names, as a data URL.
fn icon_of(folder: &Path, manifest: &Value) -> String {
    let mut choices: Vec<(u32, String)> = Vec::new();
    let mut collect = |value: &Value| match value {
        Value::Object(map) => choices.extend(
            map.iter()
                .filter_map(|(size, file)| Some((size.parse().ok()?, file.as_str()?.to_string()))),
        ),
        Value::String(file) => choices.push((0, file.clone())),
        _ => {}
    };
    collect(&manifest["icons"]);
    collect(&manifest["action"]["default_icon"]);
    collect(&manifest["browser_action"]["default_icon"]);
    choices.retain(|(size, file)| *size <= 128 && file.to_ascii_lowercase().ends_with(".png"));
    choices.sort_by_key(|(size, _)| std::cmp::Reverse(*size));
    let Some(root) = folder.canonicalize().ok() else {
        return String::new();
    };
    for (_, file) in choices {
        let Some(path) = folder
            .join(file.trim_start_matches('/'))
            .canonicalize()
            .ok()
        else {
            continue;
        };
        if !path.starts_with(&root) {
            continue;
        }
        if let Ok(bytes) = std::fs::read(&path) {
            if bytes.len() <= 24_000 {
                return format!("data:image/png;base64,{}", super::sdk::base64(&bytes));
            }
        }
    }
    String::new()
}

fn summary(extension: &Extension) -> Value {
    json!({
        "id": extension.id, "name": extension.name, "version": extension.version, "enabled": extension.enabled,
        "source": extension.source, "permissions": extension.permissions,
        "pinned": extension.pinned, "icon": extension.icon, "description": extension.description,
        "popup": !pages(extension).0.is_empty(), "options": !pages(extension).1.is_empty(),
        "contentScripts": extension.scripts.iter().map(|s| json!({"matches": s.matches, "jsBytes": s.js.len(), "cssBytes": s.css.len()})).collect::<Vec<_>>(),
        // Its background page does not run in AgentGo: features that need it
        // (pop-ups, toolbar actions, syncing) will not work.
        "limited": needs_background(Path::new(&extension.source)),
    })
}

/// The pop-up and options pages: from the record, or (for extensions added
/// before AgentGo opened pages) from the manifest.
fn pages(extension: &Extension) -> (String, String) {
    if !extension.popup.is_empty() || !extension.options.is_empty() {
        return (extension.popup.clone(), extension.options.clone());
    }
    let (manifest, _) = manifest_and_messages(Path::new(&extension.source));
    let first = |values: [&Value; 2]| {
        values
            .iter()
            .filter_map(|v| v.as_str())
            .map(|p| p.trim_start_matches('/').to_string())
            .find(|p| !p.is_empty())
            .unwrap_or_default()
    };
    (
        first([
            &manifest["action"]["default_popup"],
            &manifest["browser_action"]["default_popup"],
        ]),
        first([&manifest["options_ui"]["page"], &manifest["options_page"]]),
    )
}

/// What the extension page server (extpage.rs) needs for an enabled
/// extension: its folder, manifest, messages and saved storage.
pub fn page_parts(id: &str) -> Option<(PathBuf, Value, Value, Value)> {
    let extension = load().into_iter().find(|e| e.id == id && e.enabled)?;
    let folder = PathBuf::from(&extension.source);
    let (manifest, messages) = manifest_and_messages(&folder);
    Some((folder, manifest, messages, json!(extension.storage)))
}

/// The address of an extension's pop-up (or, with `options: true`, its
/// options page), for `extension.open_popup`.
pub fn page_url(params: &Value) -> Result<String, Fail> {
    let mut all = load();
    let extension = find(&mut all, params)?;
    if !extension.enabled {
        return Err(Fail::params(format!("{} is turned off", extension.name)));
    }
    let (popup, options) = pages(extension);
    let page = if params["options"] == true {
        options
    } else {
        popup
    };
    if page.is_empty() {
        return Err(Fail::params(format!(
            "{} has no page to open",
            extension.name
        )));
    }
    Ok(format!("agentgo-ext://{}/{page}", extension.id))
}

fn find<'a>(all: &'a mut [Extension], params: &Value) -> Result<&'a mut Extension, Fail> {
    let id = params["id"]
        .as_str()
        .ok_or_else(|| Fail::params("params.id is required; see extension.list"))?;
    all.iter_mut()
        .find(|extension| extension.id == id)
        .ok_or_else(|| Fail::params(format!("no extension {id}")))
}

/// Every extension command.
pub fn command(method: &str, params: &Value) -> Reply {
    let mut all = load();
    match method {
        "extension.list" => Ok(json!({"extensions": all.iter().map(summary).collect::<Vec<_>>()})),
        "extension.install" => {
            let folder = params["path"].as_str().ok_or_else(|| {
                Fail::params("params.path: the extension's folder (with manifest.json)")
            })?;
            let mut extension = read_folder(Path::new(folder))?;
            if extension.name == "Extension" {
                if let Some(name) = localized_name(Path::new(folder)) {
                    extension.name = name;
                }
            }
            all.retain(|known| known.id != extension.id);
            let out = summary(&extension);
            all.push(extension);
            save(&all)?;
            Ok(out)
        }
        "extension.reload" => {
            let known = find(&mut all, params)?;
            let mut fresh = read_folder(Path::new(&known.source))?;
            fresh.enabled = known.enabled;
            fresh.pinned = known.pinned;
            fresh.storage = std::mem::take(&mut known.storage);
            *known = fresh;
            let out = summary(known);
            save(&all)?;
            Ok(out)
        }
        "extension.uninstall" => {
            let before = all.len();
            let id = params["id"].as_str().unwrap_or("");
            all.retain(|known| known.id != id);
            if all.len() == before {
                return Err(Fail::params(format!("no extension {id}")));
            }
            save(&all)?;
            Ok(json!({"id": id, "uninstalled": true}))
        }
        "extension.enable" | "extension.disable" => {
            let known = find(&mut all, params)?;
            known.enabled = method == "extension.enable";
            // The toolbar button is set in the same call: {id, pinned}.
            if let Some(pinned) = params["pinned"].as_bool() {
                known.pinned = pinned;
            }
            let out = summary(known);
            save(&all)?;
            Ok(out)
        }
        "extension.permissions" => Ok(
            json!({"id": find(&mut all, params)?.id, "permissions": find(&mut all, params)?.permissions}),
        ),
        "extension.content_scripts" => {
            let known = find(&mut all, params)?;
            Ok(
                json!({"id": known.id, "scripts": known.scripts.iter().map(|s| json!({"matches": s.matches, "exclude": s.exclude, "js": s.js.chars().take(4000).collect::<String>(), "css": s.css.chars().take(2000).collect::<String>()})).collect::<Vec<_>>()}),
            )
        }
        _ => {
            // extension.storage: read, or with values, merge.
            let known = find(&mut all, params)?;
            if let Some(Value::Object(values)) = params.get("values") {
                for (key, value) in values {
                    known.storage.insert(key.clone(), value.clone());
                }
                let out = json!({"id": known.id, "storage": known.storage});
                save(&all)?;
                return Ok(out);
            }
            Ok(json!({"id": known.id, "storage": known.storage}))
        }
    }
}

/// The extension id in a Chrome Web Store address, or an id itself.
fn store_id(text: &str) -> Option<String> {
    let candidate = text
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("")
        .split(['?', '#'])
        .next()
        .unwrap_or("");
    (candidate.len() == 32 && candidate.bytes().all(|b| (b'a'..=b'p').contains(&b)))
        .then(|| candidate.to_string())
}

/// The zip archive inside a Chrome extension package (CRX version 2 or 3).
fn crx_zip(bytes: &[u8]) -> Option<&[u8]> {
    let word = |at: usize| {
        bytes
            .get(at..at + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
    };
    if bytes.get(0..4)? != b"Cr24" {
        return None;
    }
    let start = match word(4)? {
        3 => 12 + word(8)?,
        2 => 16 + word(8)? + word(12)?,
        _ => return None,
    };
    let zip = bytes.get(start..)?;
    (zip.get(0..2)? == b"PK").then_some(zip)
}

/// `extension.install {store}`: installs an extension from the Chrome Web
/// Store (its page address or its id). The package comes from Google's update
/// service, as Chrome gets it; its content scripts then run like any other
/// installed extension.
pub async fn install_from_store(store: &str) -> Reply {
    let id = store_id(store).ok_or_else(|| Fail::params("params.store: a Chrome Web Store extension page (…/detail/name/<id>) or its 32-letter id"))?;
    let url = format!(
        "https://clients2.google.com/service/update2/crx?response=redirect&prodversion={}&acceptformat=crx2,crx3&x=id%3D{id}%26uc",
        crate::CEF_CHROMIUM_VERSION
    );
    let (status, bytes) = super::connect::download(&url, 60_000_000).await?;
    if crx_zip(&bytes).is_none() {
        return Err(Fail::internal(format!(
            "the Web Store did not send an extension package (HTTP {status}, {} bytes)",
            bytes.len()
        )));
    }
    // The package must be signed by the key its id derives from.
    let zip = super::pack::verify_crx(&bytes, &id).map_err(|reason| {
        Fail::new(
            ErrorCode::PolicyDenied,
            format!("the extension package was refused: {reason}"),
        )
    })?;
    let root = state::state().options.cache_root.join("extensions");
    let folder = root.join(&id);
    std::fs::create_dir_all(&root).map_err(|error| Fail::internal(error.to_string()))?;
    let _ = std::fs::remove_dir_all(&folder);
    if let Err(reason) = super::pack::extract_zip(zip, &folder) {
        let _ = std::fs::remove_dir_all(&folder);
        return Err(Fail::internal(format!(
            "the extension package could not be unpacked: {reason}"
        )));
    }
    let mut extension = read_folder(&folder)?;
    // Names like __MSG_appName__ come from the extension's own translations.
    if extension.name == "Extension" {
        extension.name = localized_name(&folder).unwrap_or_else(|| id.clone());
    }
    extension.id = id.clone();
    let mut all = load();
    all.retain(|known| known.id != extension.id);
    let out = summary(&extension);
    all.push(extension);
    save(&all)?;
    Ok(out)
}

fn localized_name(folder: &Path) -> Option<String> {
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(folder.join("manifest.json")).ok()?).ok()?;
    let key = manifest["name"]
        .as_str()?
        .strip_prefix("__MSG_")?
        .strip_suffix("__")?
        .to_string();
    let locale = manifest["default_locale"].as_str().unwrap_or("en");
    let messages: Value = serde_json::from_slice(
        &std::fs::read(folder.join("_locales").join(locale).join("messages.json")).ok()?,
    )
    .ok()?;
    let entry = messages
        .as_object()?
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(&key))?
        .1;
    entry["message"]
        .as_str()
        .map(|name| name.chars().take(60).collect())
}

/// A Chrome match pattern (`*://*.example.com/*`, `<all_urls>`) as a test.
pub(super) fn matches(pattern: &str, url: &str) -> bool {
    if pattern == "<all_urls>" {
        return url.starts_with("http") || url.starts_with("file:");
    }
    let Some((scheme, rest)) = pattern.split_once("://") else {
        return false;
    };
    let Some((url_scheme, url_rest)) = url.split_once("://") else {
        return false;
    };
    if !(scheme == "*" && matches!(url_scheme, "http" | "https") || scheme == url_scheme) {
        return false;
    }
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let (url_host, url_path) = url_rest.split_once('/').unwrap_or((url_rest, ""));
    let url_host = url_host.split(':').next().unwrap_or("");
    let host_ok = host == "*"
        || host == url_host
        || host
            .strip_prefix("*.")
            .is_some_and(|base| url_host == base || url_host.ends_with(&format!(".{base}")));
    host_ok && glob(path, url_path.split('#').next().unwrap_or(""))
}

fn glob(pattern: &str, text: &str) -> bool {
    match pattern.split_once('*') {
        None => pattern == text,
        Some((head, tail)) => {
            text.starts_with(head)
                && (0..=text.len() - head.len()).any(|at| {
                    text.is_char_boundary(head.len() + at) && glob(tail, &text[head.len() + at..])
                })
        }
    }
}

/// The Web Store's own button asked to install `id` (webstore.js). Allowed
/// only on the store page of that very extension, after a click by the
/// person: a click that came with an agent's input is refused. (An agent
/// merely being connected does not stop the person's own click; it used to,
/// and the store then showed "Download error: undefined".)
pub fn store_request(tab: &str, id: &str) {
    let url = state::tab_url(tab);
    let on_its_page = url.starts_with("https://chromewebstore.google.com/detail/")
        && store_id(&url).as_deref() == Some(id);
    let by_person = !cdp::agent_input_recent(tab, 5_000);
    if !on_its_page || !by_person {
        state::log_event(&format!(
            "AG-SECURITY extension-install-refused {id} page={on_its_page} person={by_person}"
        ));
        state::notify_operator(if by_person {
            "The extension was not installed: install it from its own store page."
        } else {
            "The extension was not installed: only your own click can install an extension, not an agent's."
        });
        reply_to_store(tab, id, Err("refused".into()));
        return;
    }
    let (tab, id) = (tab.to_string(), id.to_string());
    super::exec::spawn(async move {
        let result = install_from_store(&id).await.map_err(|fail| fail.message);
        state::log_event(&format!(
            "AG-RUNTIME extension-install-from-store {id} ok={}",
            result.is_ok()
        ));
        if let Err(reason) = &result {
            // The reason without the folder it was checked in.
            let reason = reason.rsplit(": ").next().unwrap_or(reason);
            state::notify_operator(&format!("The extension was not installed: {reason}"));
        }
        reply_to_store(&tab, &id, result);
        state::notify_shell_state();
        state::run_in_shell("typeof refreshExtensions === 'function' && refreshExtensions()");
    });
}

fn reply_to_store(tab: &str, id: &str, result: Result<Value, String>) {
    let message = json!({
        "__agentgo": "ext-installed", "id": id, "ok": result.is_ok(),
        "name": result.as_ref().ok().and_then(|v| v["name"].as_str()).unwrap_or(""),
        "error": result.as_ref().err().cloned().unwrap_or_default(),
    });
    let tab = tab.to_string();
    super::exec::spawn(async move {
        let _ = cdp::call(
            &tab,
            "Runtime.evaluate",
            json!({"expression": format!("window.postMessage({message}, location.origin)")}),
        )
        .await;
    });
}

/// Saves storage an extension's content script changed (extshim.js).
pub fn storage_from_page(payload: &str) {
    let Ok(change) = serde_json::from_str::<Value>(payload) else {
        return;
    };
    let id = change["id"].as_str().unwrap_or("");
    let mut all = load();
    let Some(known) = all.iter_mut().find(|e| e.id == id) else {
        return;
    };
    if let Some(values) = change["set"].as_object() {
        for (key, value) in values {
            known.storage.insert(key.clone(), value.clone());
        }
    }
    for key in change["remove"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        known.storage.remove(key);
    }
    // Bounded: an extension cannot fill the profile through its storage.
    if serde_json::to_vec(&known.storage).map_or(0, |b| b.len()) <= 5 * 1024 * 1024 {
        let _ = save(&all);
    }
}

/// The extension's manifest and its messages in the default language.
fn manifest_and_messages(folder: &Path) -> (Value, Value) {
    let manifest: Value = std::fs::read(folder.join("manifest.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(Value::Null);
    let locale = manifest["default_locale"]
        .as_str()
        .unwrap_or("en")
        .to_string();
    let mut messages = serde_json::Map::new();
    for lang in [locale.as_str(), "en"] {
        let file = folder.join("_locales").join(lang).join("messages.json");
        if let Some(Value::Object(entries)) = std::fs::read(&file)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
        {
            for (key, value) in entries {
                messages.entry(key.to_lowercase()).or_insert(value);
            }
        }
    }
    (manifest, Value::Object(messages))
}

/// Whether an extension needs its background page, which AgentGo does not run.
fn needs_background(folder: &Path) -> bool {
    let (manifest, _) = manifest_and_messages(folder);
    !manifest["background"].is_null()
}

/// Runs the enabled extensions' scripts that match the tab's page. Called
/// when a document has loaded.
pub async fn inject(tab: String) {
    let url = state::tab_url(&tab);
    // The Web Store shows which extensions AgentGo already has.
    if url.starts_with("https://chromewebstore.google.com/") {
        let ids: Vec<String> = load().into_iter().map(|e| e.id).collect();
        let _ = cdp::call(
            &tab,
            "Runtime.evaluate",
            json!({"expression": format!("window.__agentgoInstalled = {}", json!(ids))}),
        )
        .await;
    }
    let mut code = Vec::new();
    for extension in load().into_iter().filter(|extension| extension.enabled) {
        for script in &extension.scripts {
            if script.matches.iter().any(|p| matches(p, &url))
                && !script.exclude.iter().any(|p| matches(p, &url))
            {
                code.push((extension.id.clone(), script.clone()));
            }
        }
    }
    if code.is_empty() {
        return;
    }
    let Ok(tree) = cdp::call(&tab, "Page.getFrameTree", json!({})).await else {
        return;
    };
    let Some(frame) = tree["frameTree"]["frame"]["id"]
        .as_str()
        .map(str::to_string)
    else {
        return;
    };
    let mut shimmed = std::collections::HashSet::new();
    for (id, script) in code {
        let Ok(world) = cdp::call(
            &tab,
            "Page.createIsolatedWorld",
            json!({"frameId": frame, "worldName": format!("agentgo-ext-{id}")}),
        )
        .await
        else {
            continue;
        };
        let Some(context) = world["executionContextId"].as_i64() else {
            continue;
        };
        // chrome.runtime, chrome.i18n and chrome.storage for its scripts.
        if shimmed.insert(id.clone()) {
            if let Some(extension) = load().into_iter().find(|e| e.id == id) {
                let (manifest, messages) = manifest_and_messages(Path::new(&extension.source));
                let _ = cdp::call(
                    &tab,
                    "Runtime.addBinding",
                    json!({"name": "__agExt", "executionContextId": context}),
                )
                .await;
                let shim = format!(
                    "{}({},{},{},{});",
                    include_str!("extshim.js"),
                    json!(id),
                    manifest,
                    messages,
                    json!(extension.storage)
                );
                let _ = cdp::call(
                    &tab,
                    "Runtime.evaluate",
                    json!({"expression": shim, "contextId": context}),
                )
                .await;
            }
        }
        let css = serde_json::to_string(&script.css).unwrap_or_default();
        let source = format!(
            "(() => {{ const css = {css}; if (css) {{ const s = document.createElement('style'); s.textContent = css; (document.head || document.documentElement).appendChild(s); }} }})();\n{}",
            script.js
        );
        let _ = cdp::call(
            &tab,
            "Runtime.evaluate",
            json!({"expression": source, "contextId": context}),
        )
        .await;
        state::log_event(&format!("AG-RUNTIME extension-ran {id} tab={tab}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_store_packages_are_unwrapped() {
        assert_eq!(store_id("https://chromewebstore.google.com/detail/dark-reader/eimadpbcbfnmbkopoojfekhnkhdbieeh?hl=en").as_deref(), Some("eimadpbcbfnmbkopoojfekhnkhdbieeh"));
        assert_eq!(
            store_id("eimadpbcbfnmbkopoojfekhnkhdbieeh").as_deref(),
            Some("eimadpbcbfnmbkopoojfekhnkhdbieeh")
        );
        assert_eq!(store_id("https://example.com/x"), None);
        let mut crx3 = b"Cr24".to_vec();
        crx3.extend(3u32.to_le_bytes());
        crx3.extend(2u32.to_le_bytes());
        crx3.extend(b"hhPKzip");
        assert_eq!(crx_zip(&crx3), Some(&b"PKzip"[..]));
        assert_eq!(crx_zip(b"PK\x03\x04"), None);
    }

    #[test]
    fn chrome_match_patterns() {
        assert!(matches("<all_urls>", "https://a.example/x"));
        assert!(matches(
            "*://*.example.com/*",
            "https://www.example.com/a?b"
        ));
        assert!(matches("*://*.example.com/*", "http://example.com/"));
        assert!(!matches(
            "*://*.example.com/*",
            "https://example.com.evil.io/"
        ));
        assert!(matches(
            "https://shop.example/items/*",
            "https://shop.example/items/42"
        ));
        assert!(!matches(
            "https://shop.example/items/*",
            "https://shop.example/cart"
        ));
        assert!(matches(
            "http://127.0.0.1/*",
            "http://127.0.0.1:8080/page.html"
        ));
        assert!(!matches("*://*/*", "file:///c:/x"));
    }
}
