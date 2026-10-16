//! The permission request box.
//!
//! Camera, microphone, location and the like stay blocked unless the person
//! says yes. A site's request waits in a queue while a themed box under the
//! address bar asks: allow this time, always allow this site, or keep it
//! blocked. No answer within a minute counts as "keep blocked".
//!
//! Everything here runs on the CEF UI thread, which is the only thread that
//! may touch the callbacks.

use std::cell::{Cell, RefCell};
use std::time::Duration;

use cef::{
    ImplMediaAccessCallback, ImplPermissionPromptCallback, MediaAccessCallback,
    PermissionPromptCallback, PermissionRequestResult,
};
use serde_json::{json, Value};

use super::{exec, extras, state};

const ANSWER_WITHIN: Duration = Duration::from_secs(60);

enum Callback {
    /// A camera or microphone request; the number is what was asked for.
    Media(MediaAccessCallback, u32),
    Prompt(PermissionPromptCallback),
}

struct Pending {
    id: u64,
    origin: String,
    kinds: Vec<&'static str>,
    callback: Callback,
}

thread_local! {
    static QUEUE: RefCell<Vec<Pending>> = const { RefCell::new(Vec::new()) };
    static NEXT: Cell<u64> = const { Cell::new(1) };
}

/// Prompts are off for automated runs, where policy alone answers.
fn prompts_off() -> bool {
    std::env::var_os("AGENTGO_NO_PROMPTS").is_some() || state::is_smoke_run()
}

/// Names of the permissions behind Chromium's request bits.
pub fn kinds_for_prompt(mask: u32) -> Vec<&'static str> {
    extras::kinds_in_mask(mask)
}

pub fn kinds_for_media(mask: u32) -> Vec<&'static str> {
    let mut kinds = Vec::new();
    if mask & 2 != 0 {
        kinds.push("camera");
    }
    if mask & 1 != 0 {
        kinds.push("microphone");
    }
    kinds
}

fn deny(pending: Pending) {
    state::log_event(&format!(
        "AG-RUNTIME permission-denied kind=prompt origin={:?} asked={:?}",
        pending.origin, pending.kinds
    ));
    match pending.callback {
        Callback::Media(callback, _) => callback.cancel(),
        Callback::Prompt(callback) => callback.cont(PermissionRequestResult::DENY),
    }
}

fn allow(pending: Pending) {
    state::log_event(&format!(
        "AG-RUNTIME permission-granted kind=prompt origin={:?} asked={:?}",
        pending.origin, pending.kinds
    ));
    match pending.callback {
        Callback::Media(callback, mask) => callback.cont(mask),
        Callback::Prompt(callback) => callback.cont(PermissionRequestResult::ACCEPT),
    }
}

/// What the box shows for the request at the front of the queue.
fn front_data() -> Value {
    QUEUE.with(|queue| {
        let queue = queue.borrow();
        match queue.first() {
            Some(front) => json!({
                "id": front.id,
                "origin": front.origin,
                "kinds": front.kinds,
                "more": queue.len() - 1,
            }),
            None => Value::Null,
        }
    })
}

/// Shows the box when nothing else is open; otherwise tries again shortly.
fn show_front() {
    if QUEUE.with(|queue| queue.borrow().is_empty()) {
        return;
    }
    if state::tool_is_free_for_prompt() {
        let _ = state::show_tool("permission", "", &front_data());
    } else {
        exec::spawn(async {
            exec::sleep(600).await;
            show_front();
        });
    }
}

/// Queues a request and asks the person. `Err` hands the callback back so
/// the caller can refuse at once (prompts are off or the origin is unusable).
fn enqueue(origin: String, kinds: Vec<&'static str>, callback: Callback) {
    if prompts_off() || kinds.is_empty() {
        deny(Pending {
            id: 0,
            origin,
            kinds,
            callback,
        });
        return;
    }
    let id = NEXT.with(|next| {
        let id = next.get();
        next.set(id + 1);
        id
    });
    QUEUE.with(|queue| {
        queue.borrow_mut().push(Pending {
            id,
            origin,
            kinds,
            callback,
        });
    });
    show_front();
    exec::spawn(async move {
        exec::sleep(ANSWER_WITHIN.as_millis() as u64).await;
        if let Some(expired) = take(id) {
            deny(expired);
            show_front();
        }
    });
}

fn take(id: u64) -> Option<Pending> {
    QUEUE.with(|queue| {
        let mut queue = queue.borrow_mut();
        let at = queue.iter().position(|pending| pending.id == id)?;
        Some(queue.remove(at))
    })
}

pub fn request_media(origin: String, asked: u32, callback: &mut MediaAccessCallback) {
    let kinds = kinds_for_media(asked);
    enqueue(origin, kinds, Callback::Media(callback.clone(), asked));
}

pub fn request_prompt(origin: String, asked: u32, callback: &mut PermissionPromptCallback) {
    let kinds = kinds_for_prompt(asked);
    enqueue(origin, kinds, Callback::Prompt(callback.clone()));
}

/// `ui.permission_answer {id, choice}`: "once", "always" or "block". Returns
/// the next request to show (or null when the queue is empty).
pub fn answer(params: &Value) -> Result<Value, String> {
    let id = params["id"]
        .as_u64()
        .ok_or("ui.permission_answer needs {id}")?;
    let choice = params["choice"].as_str().unwrap_or("block");
    let Some(pending) = take(id) else {
        return Ok(json!({"next": front_data()}));
    };
    match choice {
        "once" => allow(pending),
        "always" => {
            let grant = json!({"origin": pending.origin, "kinds": pending.kinds});
            allow(pending);
            exec::spawn(async move {
                let _ = extras::change_permission("permission.grant", &grant).await;
            });
        }
        _ => deny(pending),
    }
    let next = front_data();
    if !next.is_null() {
        let _ = state::show_tool("permission", "", &next);
    }
    Ok(json!({"next": next}))
}

/// `ui.permission_list`: the sites the person has allowed, for Settings.
pub fn list() -> Value {
    extras::list()
}

/// `ui.permission_revoke {origin}`: puts a site back to blocked.
pub async fn revoke(params: &Value) -> Result<Value, String> {
    let origin = params["origin"]
        .as_str()
        .ok_or("ui.permission_revoke needs {origin}")?;
    extras::change_permission("permission.reset", &json!({"origin": origin}))
        .await
        .map_err(|fail| format!("{fail:?}"))
}
