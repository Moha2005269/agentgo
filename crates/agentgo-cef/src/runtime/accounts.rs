//! Account Center: which of the person's accounts agents may use.
//!
//! An agent works in the person's real browser profile and may use the
//! accounts signed in there, exactly as the person can. The Account Center is
//! where the person sees those accounts and takes one away: a blocked account
//! is refused to agents on every page where it is signed in, and its API is
//! closed. Blocking, linking and adding sites are operator-only commands;
//! agents can only read the list.

use std::collections::BTreeMap;
use std::path::PathBuf;

use agentgo_accounts::{self as core, CustomSite};
use agentgo_protocol::ErrorCode;
use cef::*;
use serde_json::{json, Value};

use super::cdp;
use super::sessions::{Fail, Reply};
use super::state;

/// What the person decided, per browser profile.
#[derive(Default, serde::Serialize, serde::Deserialize)]
struct Book {
    /// Site id -> Unix time it was linked.
    #[serde(default)]
    linked: BTreeMap<String, u64>,
    /// Sites the person added to the built-in list.
    #[serde(default)]
    custom: Vec<CustomSite>,
    /// Accounts the person closed to agents.
    #[serde(default)]
    blocked: std::collections::BTreeSet<String>,
    /// Origins the person never wants a sign-in saved for.
    #[serde(default)]
    never_save: std::collections::BTreeSet<String>,
}

/// A built-in provider or an added site, in one shape.
struct Site {
    id: String,
    name: String,
    sign_in_url: String,
    /// Addresses whose cookies make up the web session.
    cookie_urls: Vec<String>,
    cookie: String,
    warning: String,
    api: Option<core::Api>,
    custom: bool,
}

pub(super) fn dir() -> PathBuf {
    state::state().options.cache_root.join("accounts")
}

fn book() -> Book {
    std::fs::read(dir().join("links.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn save(book: &Book) -> Result<(), Fail> {
    let write = || -> std::io::Result<()> {
        std::fs::create_dir_all(dir())?;
        let partial = dir().join("links.tmp");
        std::fs::write(&partial, serde_json::to_vec_pretty(book)?)?;
        std::fs::rename(partial, dir().join("links.json"))
    };
    write().map_err(|error| Fail::internal(format!("the account list could not be saved: {error}")))
}

fn origin(url: &str) -> String {
    let (scheme, rest) = url.split_once("://").unwrap_or(("https", url));
    format!(
        "{scheme}://{}/",
        rest.split(['/', '?', '#']).next().unwrap_or("")
    )
}

fn sites(book: &Book) -> Vec<Site> {
    let builtin = core::catalog().iter().map(|provider| Site {
        id: provider.id.into(),
        name: provider.name.into(),
        sign_in_url: provider.sign_in_url.into(),
        cookie_urls: std::iter::once(provider.session_cookie.0.to_string())
            .chain(
                provider
                    .hosts
                    .iter()
                    .flat_map(|host| [format!("https://{host}/"), format!("https://www.{host}/")]),
            )
            .collect(),
        cookie: provider.session_cookie.1.into(),
        warning: core::consent_warning(provider),
        api: provider.api(),
        custom: false,
    });
    let custom = book.custom.iter().map(|site| Site {
        id: site.id.clone(),
        name: site.name.clone(),
        sign_in_url: site.sign_in_url.clone(),
        cookie_urls: vec![origin(&site.sign_in_url)],
        cookie: site.cookie.clone(),
        warning: core::custom_consent_warning(site),
        api: site.api.clone(),
        custom: true,
    });
    builtin.chain(custom).collect()
}

fn site_for_host(book: &Book, host: &str) -> Option<Site> {
    let id = core::provider_for_host(host)
        .map(|provider| provider.id.to_string())
        .or_else(|| {
            book.custom
                .iter()
                .find(|site| site.owns(host))
                .map(|site| site.id.clone())
        })?;
    sites(book).into_iter().find(|site| site.id == id)
}

fn find(book: &Book, params: &Value) -> Result<Site, Fail> {
    let id = params["account"]
        .as_str()
        .ok_or_else(|| Fail::params("params.account is required"))?;
    sites(book)
        .into_iter()
        .find(|site| site.id == id)
        .ok_or_else(|| Fail::params(format!("unknown account {id}; see accounts.list")))
}

/// `accounts.list`: every account the person could link and whether agents
/// may use it. Contains no secrets.
pub fn list() -> Value {
    let book = book();
    let accounts: Vec<Value> = sites(&book)
        .iter()
        .map(|site| {
            json!({
                "account": site.id,
                "name": site.name,
                "blocked": book.blocked.contains(&site.id),
                "linked": book.linked.contains_key(&site.id),
                "linkedAt": book.linked.get(&site.id),
                "signInUrl": site.sign_in_url,
                "connectors": connector_ids(site),
                "api": site.api.as_ref().map(|_| json!({
                    "connected": super::connect::tokens(&site.id).is_some(),
                    "error": super::connect::last_error(&site.id),
                })),
                "custom": site.custom,
                "warning": site.warning,
            })
        })
        .collect();
    json!({"accounts": accounts})
}

fn connector_ids(site: &Site) -> Vec<&str> {
    site.api
        .iter()
        .flat_map(|api| api.connectors.iter().map(|connector| connector.id.as_str()))
        .collect()
}

/// Records that agents may use `id` (the API sign-in implies it).
pub(super) fn mark_linked(id: &str) -> Result<(), Fail> {
    let mut book = book();
    book.linked.entry(id.to_string()).or_insert_with(unix_now);
    save(&book)
}

/// `accounts.connect` (operator): sign in to the account's API with the
/// person's own OAuth client, after the same full-control warning.
pub fn connect(params: &Value) -> Reply {
    let site = find(&book(), params)?;
    let api = site.api.clone().ok_or_else(|| {
        Fail::params(format!(
            "{} has no API to connect; link its web session instead",
            site.name
        ))
    })?;
    super::connect::connect(&site.id, api, &site.warning, params)
}

/// `connectors.list`: the APIs an agent can call right now.
pub fn connectors_list() -> Value {
    let book = book();
    let ready: Vec<Value> = sites(&book)
        .iter()
        .filter(|site| book.linked.contains_key(&site.id) && super::connect::tokens(&site.id).is_some())
        .flat_map(|site| {
            site.api.iter().flat_map(|api| api.connectors.iter()).map(|connector| {
                json!({"account": site.id, "connector": connector.id, "name": connector.name, "apiBase": connector.api_base})
            }).collect::<Vec<_>>()
        })
        .collect();
    json!({"connectors": ready, "call": "connectors.call {account, connector, method, path, body}"})
}

/// `connectors.call`: one API request as the linked account.
pub async fn connectors_call(params: &Value) -> Reply {
    let book = book();
    let site = find(&book, params)?;
    let not_linked = || {
        Fail::new(
            ErrorCode::PolicyDenied,
            format!(
                "{} is blocked for agents or has no API; see accounts.list",
                site.name
            ),
        )
    };
    if book.blocked.contains(&site.id) {
        return Err(not_linked());
    }
    let api = site.api.as_ref().ok_or_else(not_linked)?;
    super::connect::call(&site.id, api, params).await
}

/// `accounts.block` / `accounts.unblock` (operator): close an account to
/// agents, or open it again. Agents may use every account that is not blocked.
pub fn set_blocked(params: &Value, blocked: bool) -> Reply {
    let mut book = book();
    let site = find(&book, params)?;
    if blocked {
        book.blocked.insert(site.id.clone());
    } else {
        book.blocked.remove(&site.id);
    }
    save(&book)?;
    state::log_event(&format!(
        "AG-RUNTIME account-{} {}",
        if blocked { "blocked" } else { "unblocked" },
        site.id
    ));
    state::notify_shell_state();
    Ok(json!({"account": site.id, "blocked": blocked}))
}

/// `accounts.add` (operator): add a site to the list.
pub fn add(params: &Value) -> Reply {
    let text = |key: &str| params[key].as_str().unwrap_or("");
    let mut site = CustomSite::new(text("name"), text("signInUrl"), text("cookie"))
        .map_err(|error| Fail::params(error.to_string()))?;
    if !text("authorizeUrl").is_empty() {
        site.api = Some(
            core::Api::custom(
                text("authorizeUrl"),
                text("tokenUrl"),
                text("apiBase"),
                text("scopes"),
            )
            .map_err(|error| Fail::params(error.to_string()))?,
        );
    }
    let mut book = book();
    if book.custom.iter().any(|known| known.id == site.id) {
        return Err(Fail::params("this site is already in the list"));
    }
    if book.custom.len() >= 50 {
        return Err(Fail::params("the list is limited to 50 added sites"));
    }
    book.custom.push(site);
    save(&book)?;
    state::notify_shell_state();
    Ok(list())
}

/// `accounts.link` (operator): let agents use the account. Without
/// `acknowledge: true` it only returns the warning the person must accept.
pub fn link(params: &Value) -> Reply {
    let mut book = book();
    let site = find(&book, params)?;
    if params["acknowledge"] != true {
        return Err(Fail::new(ErrorCode::ApprovalRequired, site.warning.clone())
            .with(json!({"account": site.id, "warning": site.warning, "next": "accounts.link with acknowledge:true"})));
    }
    book.linked.insert(site.id.clone(), unix_now());
    save(&book)?;
    state::log_event(&format!("AG-RUNTIME account-linked {}", site.id));
    // The person signs in themselves, in their own tab; an agent never sees
    // or types the password.
    let tab = (params["openSignIn"] != false)
        .then(|| state::create_content_tab(&site.sign_in_url).ok())
        .flatten();
    state::notify_shell_state();
    Ok(
        json!({"account": site.id, "linked": true, "tabId": tab.as_ref().map(|tab| tab["tabId"].clone())}),
    )
}

/// `accounts.unlink` (operator): take the access back. The web session of the
/// account is signed out of this browser and stored API tokens are deleted;
/// `forget: true` also removes an added site from the list.
pub fn unlink(params: &Value) -> Reply {
    let mut book = book();
    let site = find(&book, params)?;
    let was_linked = book.linked.remove(&site.id).is_some();
    if params["forget"] == true && site.custom {
        book.custom.retain(|known| known.id != site.id);
    }
    save(&book)?;
    super::connect::disconnect(&site.id, site.api.as_ref())?;
    if let Some(cookies) = state::content_context().and_then(|context| context.cookie_manager(None))
    {
        // ponytail: clears the cookies of the listed hosts (which carry the
        // session); cookies set on other subdomains stay. Sweep by domain
        // suffix with a cookie visitor if a provider proves to need it.
        for url in &site.cookie_urls {
            cookies.delete_cookies(Some(&CefString::from(url.as_str())), None, None);
        }
        cookies.flush_store(None);
    }
    state::log_event(&format!("AG-RUNTIME account-unlinked {}", site.id));
    state::notify_shell_state();
    Ok(json!({"account": site.id, "linked": false, "wasLinked": was_linked, "signedOut": true}))
}

#[cfg(windows)]
fn logins() -> core::LoginStore<core::Dpapi> {
    core::LoginStore::new(dir().join("logins.bin"), core::Dpapi)
}

thread_local! {
    /// Sign-ins seen being submitted, per tab, waiting for the person's
    /// answer. Kept in memory only.
    static OFFERS: std::cell::RefCell<std::collections::HashMap<String, core::Login>> = std::cell::RefCell::new(std::collections::HashMap::new());
    /// Origins with a saved sign-in, so the toolbar need not open the vault.
    static SAVED_ORIGINS: std::cell::RefCell<Option<Vec<String>>> = const { std::cell::RefCell::new(None) };
}

fn saved_origins() -> Vec<String> {
    SAVED_ORIGINS.with(|cache| {
        cache
            .borrow_mut()
            .get_or_insert_with(|| {
                logins()
                    .load()
                    .map(|all| all.into_iter().map(|login| login.origin).collect())
                    .unwrap_or_default()
            })
            .clone()
    })
}

fn forget_saved_origins() {
    SAVED_ORIGINS.with(|cache| *cache.borrow_mut() = None);
}

/// The saved sign-in for a page's origin (only for the browser itself, never
/// for an agent's reply).
pub fn saved_login(url: &str) -> Option<(String, String)> {
    let page = origin(url);
    let page = page.trim_end_matches('/');
    logins()
        .load()
        .ok()?
        .into_iter()
        .find(|login| login.origin == page)
        .map(|login| (login.username, login.password.to_string()))
}

/// Whether the browser can sign in on the page at `url`.
pub fn has_login(url: &str) -> bool {
    let page = origin(url);
    saved_origins()
        .iter()
        .any(|saved| saved == page.trim_end_matches('/'))
}

/// A message from the page agent (its own world, not the page).
pub fn page_report(tab: &str, payload: &str) {
    let Ok(report) = serde_json::from_str::<Value>(payload) else {
        return;
    };
    if report["kind"] != "login" || state::settings_snapshot()["offerToSavePasswords"] == false {
        return;
    }
    let (Some(username), Some(password)) =
        (report["username"].as_str(), report["password"].as_str())
    else {
        return;
    };
    let Ok(login) = core::Login::new(&state::tab_url(tab), username, password) else {
        return;
    };
    if book().never_save.contains(&login.origin)
        || logins().load().is_ok_and(|all| all.contains(&login))
    {
        return;
    }
    OFFERS.with(|offers| offers.borrow_mut().insert(tab.to_string(), login));
    if state::active_tab_id() == tab {
        let _ = state::open_tool("savelogin");
    }
    state::notify_shell_state();
}

/// The pending offer of a tab, without the password.
pub fn offer_for(tab: &str) -> Option<Value> {
    OFFERS.with(|offers| {
        offers.borrow().get(tab).map(|login| {
            json!({
                "origin": login.origin,
                "username": login.username,
                "update": saved_origins().contains(&login.origin),
            })
        })
    })
}

/// `credentials.offer_accept` / `credentials.offer_dismiss` (operator): the
/// person's answer to "save this sign-in?". `never: true` also stops asking
/// for that site.
pub fn offer_decide(params: &Value, accept: bool) -> Reply {
    let tab = params["tabId"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(state::active_tab_id);
    let login = OFFERS
        .with(|offers| offers.borrow_mut().remove(&tab))
        .ok_or_else(|| Fail::params("there is no sign-in waiting to be saved for this tab"))?;
    let origin = login.origin.clone();
    if accept {
        let store = logins();
        let mut all = store.load().map_err(vault_fail)?;
        all.retain(|known| known.origin != login.origin);
        all.push(login);
        store.save(&all).map_err(vault_fail)?;
        forget_saved_origins();
    } else if params["never"] == true {
        let mut book = book();
        book.never_save.insert(origin.clone());
        save(&book)?;
    }
    state::notify_shell_state();
    Ok(json!({"origin": origin, "saved": accept}))
}

fn vault_fail(error: core::Error) -> Fail {
    match error {
        core::Error::BadSite(_) => Fail::params(error.to_string()),
        other => Fail::internal(other.to_string()),
    }
}

/// `credentials.list`: where the browser can sign in for an agent. Never
/// contains a password.
#[cfg(windows)]
pub fn credentials_list() -> Reply {
    let saved: Vec<Value> = logins()
        .load()
        .map_err(vault_fail)?
        .iter()
        .map(|login| json!({"origin": login.origin, "username": login.username}))
        .collect();
    Ok(json!({"credentials": saved}))
}

/// `credentials.save` (operator): keep a sign-in, sealed for this Windows
/// user. One sign-in per origin; saving again replaces it.
#[cfg(windows)]
pub fn credentials_save(params: &Value) -> Reply {
    let text = |key: &str| params[key].as_str().unwrap_or("");
    let login =
        core::Login::new(text("origin"), text("username"), text("password")).map_err(vault_fail)?;
    let store = logins();
    let mut all = store.load().map_err(vault_fail)?;
    all.retain(|known| known.origin != login.origin);
    if all.len() >= 200 {
        return Err(Fail::params("the vault is limited to 200 sign-ins"));
    }
    all.push(login);
    store.save(&all).map_err(vault_fail)?;
    forget_saved_origins();
    credentials_list()
}

/// Result of [`import_logins`].
pub struct LoginImport {
    pub saved: usize,
    pub skipped: usize,
}

/// Seals sign-ins taken from another browser, one per origin, in a single
/// vault write. Entries that do not fit in the vault are counted, not lost
/// silently.
#[cfg(windows)]
pub fn import_logins(incoming: Vec<(String, String, String)>) -> LoginImport {
    let store = logins();
    let Ok(mut all) = store.load() else {
        return LoginImport {
            saved: 0,
            skipped: incoming.len(),
        };
    };
    let (mut saved, mut skipped) = (0, 0);
    for (origin, username, password) in incoming {
        let Ok(login) = core::Login::new(&origin, &username, &password) else {
            skipped += 1;
            continue;
        };
        all.retain(|known| known.origin != login.origin);
        if all.len() >= 200 {
            skipped += 1;
            continue;
        }
        all.push(login);
        saved += 1;
    }
    if store.save(&all).is_err() {
        return LoginImport {
            saved: 0,
            skipped: saved + skipped,
        };
    }
    forget_saved_origins();
    LoginImport { saved, skipped }
}

/// `credentials.delete` (operator).
#[cfg(windows)]
pub fn credentials_delete(params: &Value) -> Reply {
    let origin = params["origin"]
        .as_str()
        .ok_or_else(|| Fail::params("params.origin is required"))?;
    let store = logins();
    let mut all = store.load().map_err(vault_fail)?;
    all.retain(|known| known.origin != origin);
    store.save(&all).map_err(vault_fail)?;
    forget_saved_origins();
    credentials_list()
}

/// `credentials.fill`: the browser types the saved sign-in of the page's own
/// origin into its login form. The agent learns which fields were filled and
/// nothing else.
#[cfg(windows)]
pub async fn credentials_fill(tab: &str) -> Reply {
    let url = state::tab_url(tab);
    let page_origin = origin(&url);
    let page_origin = page_origin.trim_end_matches('/');
    let login = logins()
        .load()
        .map_err(vault_fail)?
        .into_iter()
        .find(|login| login.origin == page_origin)
        .ok_or_else(|| Fail::new(ErrorCode::PolicyDenied, format!("no sign-in is saved for {page_origin}; the person can add one in the Account Center")))?;
    let filled = super::page::agent(
        tab,
        "fillLogin",
        json!([login.username, login.password]),
        8_000,
    )
    .await?;
    if let Some(detail) = filled["error"].as_str() {
        return Err(Fail::new(
            ErrorCode::TargetStale,
            filled["detail"].as_str().unwrap_or(detail).to_string(),
        ));
    }
    Ok(
        json!({"origin": page_origin, "username": login.username, "filled": filled["filled"], "next": filled["next"]}),
    )
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

fn host_of(url: &str) -> Option<&str> {
    let (scheme, rest) = url.split_once("://")?;
    matches!(scheme, "http" | "https").then_some(())?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit('@').next()?;
    Some(host.split(':').next().unwrap_or(host))
}

/// Commands that never read or change a page.
fn pageless(method: &str) -> bool {
    const PREFIXES: [&str; 17] = [
        "session.",
        "events.",
        "agent.",
        "approval.",
        "accounts.",
        "connectors.",
        "tabs.",
        "tab.",
        "settings.",
        "window.",
        "browser.",
        "profile.",
        "bookmark.",
        "history.",
        "search.",
        "download",
        "handoff.",
    ];
    PREFIXES.iter().any(|prefix| method.starts_with(prefix)) || method == "batch"
}

/// Whether an agent's command must pass [`gate`] before it runs.
pub fn checked_before(method: &str) -> bool {
    !pageless(method) && !method.starts_with("navigate.")
}

/// Whether [`gate`] must hold after the command: it returned page content
/// or may have moved the tab to another site.
pub fn checked_after(method: &str) -> bool {
    !pageless(method)
        || matches!(
            method,
            "tabs.navigate" | "tab.navigate" | "tabs.create" | "tab.create"
        )
}

/// Refuses an agent on a page where an account it was not given is signed in.
pub async fn gate(tab: &str) -> Result<(), Fail> {
    let url = state::tab_url(tab);
    let Some(host) = host_of(&url) else {
        return Ok(());
    };
    super::suite::domain_allowed(host)?;
    let book = book();
    let Some(site) = site_for_host(&book, host) else {
        return Ok(());
    };
    if !book.blocked.contains(&site.id) {
        return Ok(());
    }
    let denied = |why: &str| {
        Fail::new(
            ErrorCode::PolicyDenied,
            format!(
                "{why} {name} is blocked for agents. The person blocked it in AgentGo's Account Center; ask them, or use a page that is not signed in.",
                name = site.name
            ),
        )
        .with(json!({"account": site.id, "blocked": true}))
    };
    // Not being able to tell counts as signed in.
    let cookies = cdp::call(tab, "Network.getCookies", json!({"urls": site.cookie_urls}))
        .await
        .map_err(|_| denied("The sign-in state of this page could not be checked and"))?;
    let signed_in = cookies["cookies"].as_array().is_some_and(|cookies| {
        cookies
            .iter()
            .any(|cookie| cookie["name"] == site.cookie.as_str())
    });
    if signed_in {
        return Err(denied("The account signed in on this page,"));
    }
    Ok(())
}
