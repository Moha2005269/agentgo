//! Side profiles: tabs with their own cookies, cache and storage, next to
//! the profile AgentGo was started with ("main").
//!
//! Each side profile is its own CEF request context. CEF only accepts a
//! context folder directly below the running profile's root, so a durable
//! side profile lives inside the profile that opened it
//! (`<root>/side-<name>`); an ephemeral or incognito one lives in memory and
//! is gone when it closes. Preferences set here (cookies, proxy, fonts,
//! permission defaults, autofill) are kept in `<data>/side-profiles.json`
//! and applied again whenever the profile opens.
//!
//! The person can lock a profile (`profile.lock`): agents then cannot open,
//! use or change it. Locking "main" keeps agents in side profiles.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;

use agentgo_protocol::ErrorCode;
use cef::{ImplDictionaryValue, ImplPreferenceManager, ImplValue};
use serde_json::{json, Value};

use super::sessions::{self, Caller, Fail, Reply};
use super::{cdp, exec, state};

struct Side {
    context: cef::RequestContext,
    ephemeral: bool,
}

thread_local! {
    static OPEN: RefCell<BTreeMap<String, Side>> = const { RefCell::new(BTreeMap::new()) };
    /// tab id -> side profile name (tabs of "main" are not listed).
    static TABS: RefCell<BTreeMap<String, String>> = const { RefCell::new(BTreeMap::new()) };
    /// session -> profile its new tabs open in.
    static SELECTED: RefCell<BTreeMap<String, String>> = const { RefCell::new(BTreeMap::new()) };
}

const MAIN: &str = "main";

fn config_file() -> PathBuf {
    state::data_dir().join("side-profiles.json")
}

fn config() -> Value {
    std::fs::read(config_file())
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}))
}

fn save_config(value: &Value) {
    let _ = std::fs::create_dir_all(state::data_dir());
    let _ = std::fs::write(
        config_file(),
        serde_json::to_vec_pretty(value).unwrap_or_default(),
    );
}

fn checked(name: &str) -> Result<String, Fail> {
    let name = name.trim().to_lowercase();
    if name.is_empty()
        || name.len() > 32
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(Fail::params(
            "a profile name is 1 to 32 letters, digits, - or _",
        ));
    }
    Ok(name)
}

fn folder(name: &str) -> PathBuf {
    state::state()
        .options
        .cache_root
        .join(format!("side-{name}"))
}

fn locked(name: &str) -> bool {
    config()["locks"]
        .as_array()
        .is_some_and(|locks| locks.iter().any(|l| l == name))
}

/// The profile a tab belongs to.
pub fn profile_of(tab: &str) -> String {
    TABS.with(|tabs| tabs.borrow().get(tab).cloned())
        .unwrap_or_else(|| MAIN.to_string())
}

/// Called for every agent tab access: agents stay out of locked profiles.
pub fn agent_may_use(tab: &str) -> Result<(), Fail> {
    let name = profile_of(tab);
    if locked(&name) {
        return Err(Fail::new(
            ErrorCode::PolicyDenied,
            format!(
                "the person locked the profile {name:?}; use profile.ephemeral or another profile"
            ),
        ));
    }
    Ok(())
}

/// Whether a saved sign-in may be typed into this tab (`profile.password_vault_binding`).
pub fn vault_allowed(tab: &str) -> Result<(), Fail> {
    let name = profile_of(tab);
    if name == MAIN || config()["vault"][&name] == true {
        return Ok(());
    }
    Err(Fail::new(
        ErrorCode::PolicyDenied,
        format!("saved sign-ins are not bound to the profile {name:?}; the person can bind them (profile.password_vault_binding)"),
    ))
}

/// Whether `tabs.create` should be served here: a profile is named or
/// selected, or "main" is locked for agents.
pub fn wants(caller: &Caller, params: &Value) -> bool {
    params.get("profile").is_some()
        || SELECTED.with(|s| s.borrow().contains_key(&caller.session))
        || (!caller.is_shell() && locked(MAIN))
}

fn to_cef(value: &Value) -> Option<cef::Value> {
    let out = cef::value_create()?;
    match value {
        Value::Bool(b) => out.set_bool(i32::from(*b)),
        Value::Number(n) => out.set_int(n.as_i64().unwrap_or(0) as i32),
        Value::String(text) => out.set_string(Some(&cef::CefString::from(text.as_str()))),
        Value::Object(map) => {
            let mut dict = cef::dictionary_value_create()?;
            for (key, item) in map {
                let key = cef::CefString::from(key.as_str());
                match item {
                    Value::Bool(b) => dict.set_bool(Some(&key), i32::from(*b)),
                    Value::Number(n) => dict.set_int(Some(&key), n.as_i64().unwrap_or(0) as i32),
                    other => dict.set_string(
                        Some(&key),
                        Some(&cef::CefString::from(other.as_str().unwrap_or(""))),
                    ),
                };
            }
            out.set_dictionary(Some(&mut dict))
        }
        _ => out.set_null(),
    };
    Some(out)
}

/// Sets one Chromium preference of a request context.
pub fn set_preference(context: &cef::RequestContext, name: &str, value: &Value) {
    let _ = apply(context, name, value);
}

fn apply(context: &cef::RequestContext, name: &str, value: &Value) -> Result<(), Fail> {
    let mut cef_value =
        to_cef(value).ok_or_else(|| Fail::internal("could not make a preference value"))?;
    let mut error = cef::CefString::from("");
    let key = cef::CefString::from(name);
    if context.set_preference(Some(&key), Some(&mut cef_value), Some(&mut error)) == 0 {
        return Err(Fail::params(format!("Chromium refused {name}: {error}")));
    }
    Ok(())
}

fn context_of(name: &str) -> Option<cef::RequestContext> {
    if name == MAIN {
        return state::content_context();
    }
    OPEN.with(|open| open.borrow().get(name).map(|side| side.context.clone()))
}

/// Sends a context's traffic through `proxy` (the VPN), or puts back the proxy
/// the profile had before. WebRTC may not reveal the real address while
/// traffic is proxied.
pub fn route_context(context: &cef::RequestContext, name: &str, proxy: Option<&Value>) {
    let value = proxy.cloned().unwrap_or_else(|| {
        let saved = &config()["prefs"][name]["proxy"];
        if saved.is_object() {
            saved.clone()
        } else {
            json!({"mode": "system"})
        }
    });
    let _ = apply(context, "proxy", &value);
    let policy = if proxy.is_some() {
        "disable_non_proxied_udp"
    } else {
        "default"
    };
    let _ = apply(context, "webrtc.ip_handling_policy", &json!(policy));
}

/// Opens (or returns) a side profile's context.
async fn open(name: &str, ephemeral: bool) -> Result<cef::RequestContext, Fail> {
    if let Some(context) = context_of(name) {
        return Ok(context);
    }
    let slot = exec::Slot::default();
    let path = (!ephemeral).then(|| folder(name));
    if let Some(path) = &path {
        std::fs::create_dir_all(path).map_err(|e| Fail::internal(e.to_string()))?;
    }
    if !super::app::create_side_context(path, slot.clone()) {
        return Err(Fail::internal("Chromium could not create the profile"));
    }
    let context = slot
        .wait(15_000)
        .await
        .ok_or_else(|| Fail::new(ErrorCode::Timeout, "the profile did not open in time"))?;
    // Preferences the profile was given before.
    if let Some(prefs) = config()["prefs"][name].as_object() {
        for (key, value) in prefs {
            let _ = apply(&context, key, value);
        }
    }
    route_context(&context, name, None);
    OPEN.with(|open| {
        open.borrow_mut().insert(
            name.to_string(),
            Side {
                context: context.clone(),
                ephemeral,
            },
        )
    });
    sessions::emit(
        "profile.opened",
        json!({"profile": name, "ephemeral": ephemeral}),
    );
    Ok(context)
}

/// `tabs.create` in a profile.
pub async fn create_tab(caller: &Caller, params: &Value) -> Reply {
    let name = match params["profile"].as_str() {
        Some(name) => checked(name)?,
        None => SELECTED
            .with(|s| s.borrow().get(&caller.session).cloned())
            .unwrap_or_else(|| MAIN.to_string()),
    };
    if !caller.is_shell() && locked(&name) {
        return Err(Fail::new(
            ErrorCode::PolicyDenied,
            format!(
                "the person locked the profile {name:?}; use profile.ephemeral or another profile"
            ),
        ));
    }
    let ephemeral = params["ephemeral"] == true
        || OPEN.with(|o| o.borrow().get(&name).is_some_and(|s| s.ephemeral));
    let context = open(&name, ephemeral).await?;
    let url = params["url"].as_str().unwrap_or("about:blank");
    let placement = if caller.is_shell() {
        state::Placement::Front
    } else {
        state::agent_tab_placement()
    };
    let mut out = state::create_content_tab_in(url, placement, context).map_err(Fail::params)?;
    let tab = out["tabId"].as_str().unwrap_or_default().to_string();
    if name != MAIN {
        TABS.with(|tabs| tabs.borrow_mut().insert(tab.clone(), name.clone()));
    }
    sessions::authorize_tab(caller, &tab)?;
    out["profile"] = json!(name);
    Ok(out)
}

fn tabs_of(name: &str) -> Vec<String> {
    state::content_tab_ids()
        .into_iter()
        .filter(|tab| profile_of(tab) == name)
        .collect()
}

fn describe(name: &str) -> Value {
    let open = context_of(name).is_some();
    let ephemeral = OPEN.with(|o| o.borrow().get(name).is_some_and(|s| s.ephemeral));
    json!({
        "profile": name,
        "open": open,
        "ephemeral": ephemeral,
        "locked": locked(name),
        "path": if name == MAIN { state::state().options.cache_root.join("content").display().to_string() } else if ephemeral { "memory".into() } else { folder(name).display().to_string() },
        "tabs": tabs_of(name),
        "savedSignIns": name == MAIN || config()["vault"][name] == true,
        "preferences": config()["prefs"][name],
    })
}

fn copy_dir(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<u64> {
    std::fs::create_dir_all(to)?;
    let mut count = 0;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            count += copy_dir(&entry.path(), &target)?;
        } else if std::fs::copy(entry.path(), &target).is_ok() {
            // Lock files of a profile that is in use elsewhere are skipped.
            count += 1;
        }
    }
    Ok(count)
}

/// Only the person changes "main" and locked profiles.
fn may_change(caller: &Caller, name: &str) -> Result<(), Fail> {
    if !caller.is_shell() && (name == MAIN || locked(name)) {
        return Err(Fail::new(
            ErrorCode::PolicyDenied,
            format!("only the person changes the profile {name:?}"),
        ));
    }
    Ok(())
}

fn name_param(params: &Value) -> Result<String, Fail> {
    checked(
        params["profile"]
            .as_str()
            .or(params["name"].as_str())
            .unwrap_or(MAIN),
    )
}

/// Remembers a preference for a profile and applies it if it is open.
fn set_pref(name: &str, key: &str, value: Value) -> Result<(), Fail> {
    if let Some(context) = context_of(name) {
        apply(&context, key, &value)?;
    }
    let mut all = config();
    all["prefs"][name][key] = value;
    save_config(&all);
    Ok(())
}

pub fn handles(method: &str) -> bool {
    method.starts_with("profile.")
        && matches!(
            &method[8..],
            "open"
                | "close"
                | "select"
                | "lock"
                | "unlock"
                | "clone"
                | "ephemeral"
                | "incognito"
                | "reset_site"
                | "cookie_policy"
                | "storage_partition"
                | "accessibility_preferences"
                | "proxy"
                | "permission_defaults"
                | "autofill_policy"
                | "password_vault_binding"
        )
        || method == "appearance.fonts"
}

pub async fn command(caller: &Caller, method: &str, params: &Value) -> Reply {
    match method {
        "profile.open" => {
            let name = name_param(params)?;
            create_tab(
                caller,
                &json!({"profile": name, "url": params["url"], "ephemeral": params["ephemeral"]}),
            )
            .await
        }
        "profile.ephemeral" | "profile.incognito" => {
            let name = format!(
                "{}-{:x}",
                if method == "profile.incognito" {
                    "incognito"
                } else {
                    "ephemeral"
                },
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_millis() as u64)
                    & 0xffffff
            );
            create_tab(
                caller,
                &json!({"profile": name, "url": params["url"], "ephemeral": true}),
            )
            .await
        }
        "profile.select" => {
            let name = name_param(params)?;
            if !caller.is_shell() && locked(&name) {
                return Err(Fail::new(
                    ErrorCode::PolicyDenied,
                    format!("the person locked the profile {name:?}"),
                ));
            }
            SELECTED.with(|s| {
                if name == MAIN {
                    s.borrow_mut().remove(&caller.session)
                } else {
                    s.borrow_mut().insert(caller.session.clone(), name.clone())
                }
            });
            Ok(json!({"selected": name, "note": "tabs.create without profile now opens here"}))
        }
        "profile.close" => {
            let name = name_param(params)?;
            if name == MAIN {
                return Err(Fail::params(
                    "the main profile closes with the window (browser.shutdown)",
                ));
            }
            let mut closed = Vec::new();
            for tab in tabs_of(&name) {
                if caller.is_shell()
                    || sessions::lease_holder(&tab).is_none_or(|holder| holder == caller.session)
                {
                    if state::close_content_tab(&tab).is_ok() {
                        closed.push(tab);
                    }
                } else {
                    return Err(Fail::new(
                        ErrorCode::Unauthorized,
                        format!("tab {tab} of {name} belongs to another agent"),
                    ));
                }
            }
            let was_open = OPEN.with(|o| o.borrow_mut().remove(&name)).is_some();
            TABS.with(|t| t.borrow_mut().retain(|_, p| *p != name));
            SELECTED.with(|s| s.borrow_mut().retain(|_, p| *p != name));
            Ok(json!({"profile": name, "closed": was_open, "tabs": closed}))
        }
        "profile.lock" | "profile.unlock" => {
            if !caller.is_shell() {
                return Err(Fail::new(
                    ErrorCode::PolicyDenied,
                    "locking profiles is the person's decision",
                ));
            }
            let name = name_param(params)?;
            let mut all = config();
            let mut locks: Vec<Value> = all["locks"].as_array().cloned().unwrap_or_default();
            locks.retain(|l| l != name.as_str());
            if method == "profile.lock" {
                locks.push(json!(name));
            }
            all["locks"] = json!(locks);
            save_config(&all);
            Ok(json!({"profile": name, "locked": method == "profile.lock"}))
        }
        "profile.clone" => {
            let from = name_param(params)?;
            let to = checked(
                params["as"]
                    .as_str()
                    .ok_or_else(|| Fail::params("as (the new profile's name) is required"))?,
            )?;
            if from == MAIN {
                return Err(Fail::params(
                    "the main profile is in use; clone side profiles (profile.close them first)",
                ));
            }
            if context_of(&from).is_some() {
                return Err(Fail::params(format!(
                    "close {from} first (profile.close): Chromium holds its files while it is open"
                )));
            }
            if !folder(&from).exists() {
                return Err(Fail::params(format!("there is no saved profile {from}")));
            }
            if folder(&to).exists() {
                return Err(Fail::params(format!("a profile named {to} exists already")));
            }
            let files = copy_dir(&folder(&from), &folder(&to))
                .map_err(|e| Fail::internal(e.to_string()))?;
            let mut all = config();
            all["prefs"][&to] = all["prefs"][&from].clone();
            save_config(&all);
            Ok(json!({"profile": to, "from": from, "files": files}))
        }
        "profile.reset_site" => {
            let name = name_param(params)?;
            may_change(caller, &name)?;
            let origin = params["origin"]
                .as_str()
                .ok_or_else(|| Fail::params("origin is required"))?;
            let origin = super::dispatch::web_origin(origin).ok_or_else(|| {
                Fail::params("origin must be an address such as https://example.com")
            })?;
            let tab = match tabs_of(&name).into_iter().next() {
                Some(tab) => tab,
                None => {
                    return Err(Fail::params(format!(
                        "open a tab of {name} first (profile.open)"
                    )))
                }
            };
            cdp::call(
                &tab,
                "Storage.clearDataForOrigin",
                json!({"origin": origin, "storageTypes": "all"}),
            )
            .await
            .map_err(Fail::internal)?;
            Ok(json!({"profile": name, "origin": origin, "cleared": true}))
        }
        "profile.cookie_policy" => {
            let name = name_param(params)?;
            may_change(caller, &name)?;
            let policy = params["policy"].as_str().unwrap_or("");
            let (mode, default) = match policy {
                "all" => (0, 1),
                "block_third_party" => (1, 1),
                "block_all" => (1, 2),
                _ => return Err(Fail::params("policy: all, block_third_party or block_all")),
            };
            set_pref(&name, "profile.cookie_controls_mode", json!(mode))?;
            set_pref(
                &name,
                "profile.default_content_setting_values.cookies",
                json!(default),
            )?;
            Ok(json!({"profile": name, "policy": policy}))
        }
        "profile.proxy" => {
            let name = name_param(params)?;
            may_change(caller, &name)?;
            let value = match params["mode"]
                .as_str()
                .unwrap_or(if params.get("server").is_some() {
                    "fixed_servers"
                } else {
                    ""
                }) {
                "direct" => json!({"mode": "direct"}),
                "system" => json!({"mode": "system"}),
                "fixed_servers" => {
                    let server = params["server"].as_str().ok_or_else(|| {
                        Fail::params("server, such as 127.0.0.1:8080 or socks5://host:1080")
                    })?;
                    json!({"mode": "fixed_servers", "server": server, "bypass_list": params["bypass"].as_str().unwrap_or("<local>")})
                }
                "pac_script" => {
                    json!({"mode": "pac_script", "pac_url": params["pacUrl"].as_str().ok_or_else(|| Fail::params("pacUrl is required"))?})
                }
                _ => return Err(Fail::params(
                    "mode: direct, system, fixed_servers (server, bypass) or pac_script (pacUrl)",
                )),
            };
            set_pref(&name, "proxy", value.clone())?;
            Ok(json!({"profile": name, "proxy": value}))
        }
        "profile.accessibility_preferences" => {
            let name = name_param(params)?;
            let mut set = serde_json::Map::new();
            for (param, pref) in [
                ("fontSize", "webkit.webprefs.default_font_size"),
                ("minimumFontSize", "webkit.webprefs.minimum_font_size"),
                ("fixedFontSize", "webkit.webprefs.default_fixed_font_size"),
            ] {
                if let Some(size) = params[param].as_u64() {
                    may_change(caller, &name)?;
                    set_pref(&name, pref, json!(size.clamp(6, 72)))?;
                    set.insert(param.into(), json!(size.clamp(6, 72)));
                }
            }
            Ok(json!({"profile": name, "set": set, "preferences": config()["prefs"][&name]}))
        }
        "appearance.fonts" => {
            let name = name_param(params)?;
            let mut set = serde_json::Map::new();
            for (param, pref) in [
                ("standard", "webkit.webprefs.fonts.standard.Zyyy"),
                ("serif", "webkit.webprefs.fonts.serif.Zyyy"),
                ("sansSerif", "webkit.webprefs.fonts.sansserif.Zyyy"),
                ("fixed", "webkit.webprefs.fonts.fixed.Zyyy"),
            ] {
                if let Some(family) = params[param].as_str() {
                    may_change(caller, &name)?;
                    let family: String = family
                        .chars()
                        .filter(|c| c.is_alphanumeric() || " -_".contains(*c))
                        .take(64)
                        .collect();
                    set_pref(&name, pref, json!(family))?;
                    set.insert(param.into(), json!(family));
                }
            }
            if let Some(size) = params["size"].as_u64() {
                may_change(caller, &name)?;
                set_pref(
                    &name,
                    "webkit.webprefs.default_font_size",
                    json!(size.clamp(6, 72)),
                )?;
                set.insert("size".into(), json!(size.clamp(6, 72)));
            }
            Ok(json!({"profile": name, "set": set, "preferences": config()["prefs"][&name]}))
        }
        "profile.permission_defaults" => {
            let name = name_param(params)?;
            may_change(caller, &name)?;
            let kind = params["kind"].as_str().unwrap_or("");
            let key = match kind {
                "geolocation" | "notifications" | "popups" | "images" | "javascript" | "sound" | "automatic_downloads" | "clipboard" => kind.to_string(),
                "camera" => "media_stream_camera".into(),
                "microphone" => "media_stream_mic".into(),
                _ => return Err(Fail::params("kind: geolocation, notifications, popups, images, javascript, sound, automatic_downloads, clipboard, camera or microphone")),
            };
            let value = match params["setting"].as_str() {
                Some("block") => 2,
                Some("ask") => 3,
                Some("allow") if matches!(kind, "popups" | "images" | "javascript" | "sound" | "automatic_downloads") => 1,
                Some("allow") => return Err(Fail::new(ErrorCode::PolicyDenied, "sites get camera, location and similar permissions only from the person, site by site (permission.grant)")),
                _ => return Err(Fail::params("setting: block, ask or allow")),
            };
            set_pref(
                &name,
                &format!("profile.default_content_setting_values.{key}"),
                json!(value),
            )?;
            Ok(json!({"profile": name, "kind": kind, "setting": params["setting"]}))
        }
        "profile.autofill_policy" => {
            let name = name_param(params)?;
            may_change(caller, &name)?;
            let on = params["enabled"]
                .as_bool()
                .ok_or_else(|| Fail::params("enabled (true or false) is required"))?;
            set_pref(&name, "autofill.profile_enabled", json!(on))?;
            set_pref(&name, "autofill.credit_card_enabled", json!(false))?;
            Ok(
                json!({"profile": name, "addresses": on, "cards": false, "note": "card autofill stays off: payment fields always go to the person"}),
            )
        }
        "profile.password_vault_binding" => {
            let name = name_param(params)?;
            if params.get("bind").is_none() {
                return Ok(
                    json!({"profile": name, "bound": name == MAIN || config()["vault"][&name] == true}),
                );
            }
            if !caller.is_shell() {
                return Err(Fail::new(
                    ErrorCode::PolicyDenied,
                    "binding saved sign-ins to a profile is the person's decision",
                ));
            }
            let mut all = config();
            all["vault"][&name] = json!(params["bind"] == true);
            save_config(&all);
            Ok(json!({"profile": name, "bound": params["bind"] == true}))
        }
        "profile.storage_partition" => match params["profile"].as_str().or(params["name"].as_str())
        {
            Some(name) => Ok(describe(&checked(name)?)),
            None => {
                let mut names = vec![MAIN.to_string()];
                names.extend(OPEN.with(|o| o.borrow().keys().cloned().collect::<Vec<_>>()));
                for entry in std::fs::read_dir(&state::state().options.cache_root)
                    .into_iter()
                    .flatten()
                    .flatten()
                {
                    if let Some(name) = entry.file_name().to_string_lossy().strip_prefix("side-") {
                        if !names.iter().any(|n| n == name) {
                            names.push(name.to_string());
                        }
                    }
                }
                Ok(json!({"profiles": names.iter().map(|n| describe(n)).collect::<Vec<_>>()}))
            }
        },
        _ => Err(Fail::internal(format!("{method} is not a profile command"))),
    }
}
