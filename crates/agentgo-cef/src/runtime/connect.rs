//! API side of the Account Center: OAuth sign-in on a loopback redirect and
//! the gateway through which agents call a linked account's APIs.
//!
//! An agent never holds a token. It names an account, a connector and a path;
//! the browser checks the path stays inside that API, attaches the token and
//! returns the answer. Requests go through Chromium's network stack without
//! the profile's cookies and never follow a redirect, so a token cannot be
//! carried to another host.

use std::cell::RefCell;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::rc::Rc;
use std::time::{Duration, Instant};

use agentgo_accounts::{self as core, Api, Attempt, Tokens};
use agentgo_protocol::ErrorCode;
use cef::*;
use serde_json::{json, Value};

use super::exec::{self, Slot};
use super::sessions::{Fail, Reply};
use super::state;

/// Largest answer kept from one request.
const BODY_LIMIT: usize = 2_000_000;
/// DISABLE_CACHE | STOP_ON_REDIRECT (cef_urlrequest_flags_t).
const REQUEST_FLAGS: i32 = (1 << 2) | (1 << 7);

type HttpResult = (i32, Vec<u8>);

wrap_urlrequest_client! {
    struct HttpClient {
        body: Rc<RefCell<Vec<u8>>>,
        done: Slot<HttpResult>,
        limit: usize,
    }

    impl UrlrequestClient {
        fn on_download_data(
            &self,
            _request: Option<&mut Urlrequest>,
            data: *const u8,
            data_length: usize,
        ) {
            let mut body = self.body.borrow_mut();
            if !data.is_null() && body.len() + data_length <= self.limit {
                // SAFETY: CEF passes `data_length` readable bytes at `data`
                // for the duration of this call.
                body.extend_from_slice(unsafe { std::slice::from_raw_parts(data, data_length) });
            }
        }

        fn on_request_complete(&self, request: Option<&mut Urlrequest>) {
            let status = request
                .and_then(|request| request.response())
                .map_or(0, |response| response.status());
            self.done.fill((status, std::mem::take(&mut *self.body.borrow_mut())));
        }
    }
}

/// One request. Returns the HTTP status (0 when the request failed or was
/// redirected) and the body.
async fn http(
    method: &str,
    url: &str,
    headers: &[(&str, &str)],
    body: Option<(&str, &[u8])>,
) -> Result<HttpResult, Fail> {
    request(
        method,
        url,
        headers,
        body,
        REQUEST_FLAGS,
        BODY_LIMIT,
        30_000,
    )
    .await
}

/// A plain download that follows redirects (no account token is involved),
/// for packages such as Chrome Web Store extensions.
pub(super) async fn download(url: &str, limit: usize) -> Result<HttpResult, Fail> {
    request("GET", url, &[], None, 1 << 2, limit, 120_000).await
}

#[allow(clippy::too_many_arguments)]
async fn request(
    method: &str,
    url: &str,
    headers: &[(&str, &str)],
    body: Option<(&str, &[u8])>,
    flags: i32,
    limit: usize,
    timeout_ms: u64,
) -> Result<HttpResult, Fail> {
    let request = request_create().ok_or_else(|| Fail::internal("could not create a request"))?;
    request.set_url(Some(&CefString::from(url)));
    request.set_method(Some(&CefString::from(method)));
    request.set_flags(flags);
    for (name, value) in headers {
        request.set_header_by_name(
            Some(&CefString::from(*name)),
            Some(&CefString::from(*value)),
            1,
        );
    }
    if let Some((content_type, bytes)) = body {
        let (Some(mut data), Some(mut element)) = (post_data_create(), post_data_element_create())
        else {
            return Err(Fail::internal("could not create the request body"));
        };
        element.set_to_bytes(bytes.len(), bytes.as_ptr());
        data.add_element(Some(&mut element));
        request.set_post_data(Some(&mut data));
        request.set_header_by_name(
            Some(&CefString::from("Content-Type")),
            Some(&CefString::from(content_type)),
            1,
        );
    }
    let done = Slot::default();
    let mut client = HttpClient::new(Rc::new(RefCell::new(Vec::new())), done.clone(), limit);
    let mut request = request;
    // The handle keeps the request alive until it completes.
    let _running = urlrequest_create(Some(&mut request), Some(&mut client), None)
        .ok_or_else(|| Fail::internal("the request could not be started"))?;
    done.wait(timeout_ms)
        .await
        .ok_or_else(|| Fail::internal(format!("no answer within {} seconds", timeout_ms / 1000)))
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

fn vault() -> core::Vault<core::Dpapi> {
    core::Vault::new(super::accounts::dir(), core::Dpapi)
}

fn internal(error: core::Error) -> Fail {
    Fail::internal(error.to_string())
}

/// The stored tokens of an account, if its API is connected.
pub fn tokens(account: &str) -> Option<Tokens> {
    vault().load(account).ok().flatten()
}

struct Pending {
    account: String,
    attempt: Attempt,
    client_id: String,
    client_secret: Option<String>,
}

thread_local! {
    /// The one sign-in in progress. Starting another replaces it, which makes
    /// the earlier answer worthless.
    static PENDING: RefCell<Option<Pending>> = const { RefCell::new(None) };
    /// Outcome of the last attempt per account, for the Account Center panel.
    static OUTCOME: RefCell<std::collections::HashMap<String, String>> = RefCell::new(std::collections::HashMap::new());
}

/// Why the last API sign-in of `account` failed, if it did.
pub fn last_error(account: &str) -> Option<String> {
    OUTCOME.with(|all| all.borrow().get(account).cloned())
}

/// Serves the redirect on `listener`: answers stray requests with 404 until
/// the callback arrives or five minutes pass, then hands the callback's Host
/// header and request target to the UI thread.
fn serve_redirect(listener: TcpListener) {
    let deadline = Instant::now() + Duration::from_secs(300);
    let _ = listener.set_nonblocking(true);
    while Instant::now() < deadline {
        let Ok((mut stream, peer)) = listener.accept() else {
            std::thread::sleep(Duration::from_millis(50));
            continue;
        };
        if !peer.ip().is_loopback() {
            continue;
        }
        let _ = stream.set_nonblocking(false);
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        let mut seen = Vec::new();
        let mut chunk = [0_u8; 2048];
        while seen.len() < 16_384 && !seen.windows(4).any(|window| window == b"\r\n\r\n") {
            match stream.read(&mut chunk) {
                Ok(count) if count > 0 => seen.extend_from_slice(&chunk[..count]),
                _ => break,
            }
        }
        let head = String::from_utf8_lossy(&seen);
        let mut lines = head.split("\r\n");
        let target = lines
            .next()
            .and_then(|line| line.strip_prefix("GET "))
            .and_then(|rest| rest.split(' ').next())
            .unwrap_or("")
            .to_string();
        let host = lines
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("host")
                    .then(|| value.trim().to_string())
            })
            .unwrap_or_default();
        let callback = target.split('?').next() == Some("/callback");
        let page = if callback {
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n<!doctype html><meta charset=utf-8><title>AgentGo</title><body style=\"font:16px system-ui;margin:3em\"><h1>You can close this tab</h1><p>AgentGo received the answer. The Account Center shows whether the account is connected.</p>"
        } else {
            "HTTP/1.1 404 Not Found\r\nConnection: close\r\nContent-Length: 0\r\n\r\n"
        };
        let _ = stream.write_all(page.as_bytes());
        if callback {
            exec::post(move || exec::spawn(finish(host, target)));
            state::wake_ui();
            return;
        }
    }
}

/// `accounts.connect` (operator): start the API sign-in of an account with
/// the person's own OAuth client.
pub fn connect(account: &str, api: Api, warning: &str, params: &Value) -> Reply {
    if params["acknowledge"] != true {
        return Err(Fail::new(ErrorCode::ApprovalRequired, warning.to_string()).with(json!({
            "account": account, "warning": warning, "next": "accounts.connect with acknowledge:true",
        })));
    }
    let client_id = params["clientId"].as_str().unwrap_or("").trim();
    let client_secret = params["clientSecret"]
        .as_str()
        .map(str::trim)
        .filter(|secret| !secret.is_empty())
        .map(str::to_string);
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .map_err(|error| Fail::internal(format!("could not open the sign-in listener: {error}")))?;
    let port = listener
        .local_addr()
        .map_err(|error| Fail::internal(error.to_string()))?
        .port();
    let attempt =
        Attempt::with_api(api, client_id, port).map_err(|error| Fail::params(error.to_string()))?;
    let (url, redirect) = (attempt.authorize_url(), attempt.redirect_uri());
    std::thread::Builder::new()
        .name("agentgo-oauth-redirect".into())
        .spawn(move || serve_redirect(listener))
        .map_err(|error| Fail::internal(error.to_string()))?;
    PENDING.with(|pending| {
        *pending.borrow_mut() = Some(Pending {
            account: account.to_string(),
            attempt,
            client_id: client_id.to_string(),
            client_secret,
        });
    });
    OUTCOME.with(|all| all.borrow_mut().remove(account));
    // The person signs in and consents on the provider's own page.
    let tab = state::create_content_tab(&url).map_err(Fail::internal)?;
    Ok(json!({"account": account, "started": true, "redirectUri": redirect, "tabId": tab["tabId"]}))
}

/// Runs on the UI thread when the redirect arrived.
async fn finish(host: String, target: String) {
    let Some(pending) = PENDING.with(|pending| pending.borrow_mut().take()) else {
        return;
    };
    let account = pending.account.clone();
    let outcome = exchange(pending, &host, &target).await;
    match &outcome {
        Ok(()) => state::log_event(&format!("AG-RUNTIME account-api-connected {account}")),
        Err(fail) => {
            state::log_event(&format!("AG-RUNTIME account-api-refused {account}"));
            OUTCOME.with(|all| {
                all.borrow_mut()
                    .insert(account.clone(), fail.message.clone())
            });
        }
    }
    state::notify_shell_state();
}

async fn exchange(pending: Pending, host: &str, target: &str) -> Result<(), Fail> {
    let refused = |error: core::Error| Fail::new(ErrorCode::PolicyDenied, error.to_string());
    let request = pending
        .attempt
        .finish(host, target, pending.client_secret.as_deref())
        .map_err(refused)?;
    let (status, body) = http(
        "POST",
        &request.url,
        &[("Accept", "application/json")],
        Some(("application/x-www-form-urlencoded", request.body.as_bytes())),
    )
    .await?;
    if !(200..300).contains(&status) && body.is_empty() {
        return Err(Fail::internal(format!(
            "the token endpoint answered {status}"
        )));
    }
    let mut tokens = core::parse_token_response(
        &String::from_utf8_lossy(&body),
        request.scopes,
        unix_now(),
        None,
    )
    .map_err(refused)?;
    tokens.client_id = pending.client_id;
    tokens.client_secret = pending.client_secret;
    vault().save(&pending.account, &tokens).map_err(internal)?;
    super::accounts::mark_linked(&pending.account)
}

/// A usable access token for `account`, refreshed first when it is stale.
async fn fresh_tokens(account: &str, api: &Api, force: bool) -> Result<Tokens, Fail> {
    let tokens = vault().load(account).map_err(internal)?.ok_or_else(|| {
        Fail::new(
            ErrorCode::PolicyDenied,
            format!("the API of {account} is not connected; the person can connect it in the Account Center"),
        )
    })?;
    if !(force || tokens.stale(unix_now())) || tokens.refresh_token.is_none() {
        return Ok(tokens);
    }
    let request = core::refresh_request(api, &tokens).map_err(internal)?;
    let (_, body) = http(
        "POST",
        &request.url,
        &[("Accept", "application/json")],
        Some(("application/x-www-form-urlencoded", request.body.as_bytes())),
    )
    .await?;
    let renewed = core::parse_token_response(
        &String::from_utf8_lossy(&body),
        request.scopes,
        unix_now(),
        Some(&tokens),
    )
    .map_err(|error| {
        Fail::new(
            ErrorCode::PolicyDenied,
            format!(
                "the access to {account} has ended ({error}); the person must connect it again"
            ),
        )
    })?;
    vault().save(account, &renewed).map_err(internal)?;
    Ok(renewed)
}

/// `connectors.call`: one request to a linked account's API.
pub async fn call(account: &str, api: &Api, params: &Value) -> Reply {
    let connector = params["connector"]
        .as_str()
        .ok_or_else(|| Fail::params("params.connector is required; see connectors.list"))?;
    let method = params["method"]
        .as_str()
        .unwrap_or("GET")
        .to_ascii_uppercase();
    let path = params["path"]
        .as_str()
        .ok_or_else(|| Fail::params("params.path is required, for example /files?pageSize=10"))?;
    let url = core::api_url(api, connector, &method, path)
        .map_err(|error| Fail::params(error.to_string()))?;
    let body = match params.get("body") {
        None | Some(Value::Null) => None,
        Some(Value::String(text)) => Some((
            params["contentType"]
                .as_str()
                .unwrap_or("text/plain; charset=utf-8")
                .to_string(),
            text.clone().into_bytes(),
        )),
        Some(other) => Some((
            "application/json".to_string(),
            other.to_string().into_bytes(),
        )),
    };
    if body.as_ref().is_some_and(|(kind, bytes)| {
        bytes.len() > 1_000_000 || kind.len() > 100 || kind.bytes().any(|b| b < b' ')
    }) {
        return Err(Fail::params("the request body is limited to 1 MB"));
    }
    let mut tokens = fresh_tokens(account, api, false).await?;
    let mut attempt = 0;
    let (status, answer) = loop {
        let bearer = format!("Bearer {}", tokens.access_token);
        let headers = [
            ("Authorization", bearer.as_str()),
            ("Accept", "application/json"),
        ];
        let sent = http(
            &method,
            &url,
            &headers,
            body.as_ref()
                .map(|(kind, bytes)| (kind.as_str(), bytes.as_slice())),
        )
        .await?;
        // One retry with a renewed token when the service says it expired.
        if sent.0 == 401 && attempt == 0 && tokens.refresh_token.is_some() {
            attempt += 1;
            tokens = fresh_tokens(account, api, true).await?;
            continue;
        }
        break sent;
    };
    if status == 0 {
        return Err(Fail::internal(
            "the request failed or the API tried to redirect it elsewhere",
        ));
    }
    let text = String::from_utf8_lossy(&answer);
    let truncated = text.chars().count() > 200_000;
    let shown: String = text.chars().take(200_000).collect();
    let body = if truncated {
        None
    } else {
        serde_json::from_str::<Value>(&shown).ok()
    };
    Ok(json!({
        "status": status,
        "ok": (200..300).contains(&status),
        "json": body,
        "text": if body.is_none() { Some(shown) } else { None },
        "truncated": truncated,
    }))
}

/// Tells the provider to cancel the tokens (when it has an endpoint for it)
/// and deletes them here. Deleting never depends on the provider answering.
pub fn disconnect(account: &str, api: Option<&Api>) -> Result<(), Fail> {
    let held = vault().load(account).ok().flatten();
    vault().remove(account).map_err(internal)?;
    if let (Some(tokens), Some(revoke)) = (held, api.and_then(|api| api.revoke.clone())) {
        exec::spawn(async move {
            let token = tokens.refresh_token.unwrap_or(tokens.access_token);
            let body = format!("token={token}");
            let _ = http(
                "POST",
                &revoke,
                &[],
                Some(("application/x-www-form-urlencoded", body.as_bytes())),
            )
            .await;
        });
    }
    Ok(())
}
