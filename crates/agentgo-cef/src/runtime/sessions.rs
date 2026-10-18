//! Agent sessions, tab leases, operator control (pause/stop), approvals and
//! human handoff. UI-thread state shared by every transport, so stdio, the
//! named pipe and the trusted shell can never disagree about policy.

use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::sync::mpsc::Sender;

use agentgo_protocol::ErrorCode;
use serde_json::{json, Value};

use super::exec;

/// Typed failure returned by command handlers.
#[derive(Debug, Clone)]
pub struct Fail {
    /// Stable protocol error code.
    pub code: ErrorCode,
    /// Human/agent readable message with a recovery hint where possible.
    pub message: String,
    /// Optional structured detail (approval id, handoff id, candidates).
    pub data: Option<Value>,
}

impl Fail {
    /// Builds a failure without structured data.
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    /// Invalid-parameter failure.
    pub fn params(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidParams, message)
    }

    /// Internal failure (browser/DevTools error surfaced honestly).
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Internal, message)
    }

    /// Attaches structured data.
    pub fn with(mut self, data: Value) -> Self {
        self.data = Some(data);
        self
    }
}

/// Result type of every command handler.
pub type Reply = Result<Value, Fail>;

/// Who issued a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Caller {
    /// `shell` for the trusted browser UI, otherwise an agent session id.
    pub session: String,
}

impl Caller {
    /// The trusted browser chrome (the human operator).
    pub fn shell() -> Self {
        Self {
            session: "shell".into(),
        }
    }

    /// Whether this is the human operator's own UI.
    pub fn is_shell(&self) -> bool {
        self.session == "shell"
    }
}

/// Operator control over agent input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    /// Agents may act.
    Running,
    /// Agent actions wait; sessions, leases and checkpoints are preserved.
    Paused,
    /// Agent input is revoked until the operator explicitly re-enables it.
    Stopped,
}

impl Control {
    fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Paused => "paused",
            Self::Stopped => "stopped",
        }
    }
}

struct Session {
    client: String,
    transport: &'static str,
    out: Sender<String>,
    subscribed: bool,
    connected_at: u64,
    actions: u64,
    last: String,
    /// Tab this session used most recently: its default target.
    current: Option<String>,
}

#[derive(Clone)]
struct Approval {
    session: String,
    tab_id: String,
    origin: String,
    target: String,
    reason: String,
    status: &'static str,
    created: u64,
    /// When the person decided; a granted approval is good for five minutes.
    decided: u64,
}

/// Seconds a pending approval waits for the person, and a granted one stays
/// usable (`policy.approval_expire`).
const APPROVAL_SECONDS: u64 = 300;

/// Pending approvals one session may have at a time.
const MAX_PENDING: usize = 20;

struct State {
    control: Control,
    sessions: BTreeMap<String, Session>,
    /// tab id -> owning session id.
    leases: BTreeMap<String, String>,
    approvals: BTreeMap<String, Approval>,
    /// tab id -> (handoff id, reason).
    handoffs: BTreeMap<String, (String, String)>,
    /// tab id -> client name: tabs kept for that client across reconnects.
    locks: BTreeMap<String, String>,
    activity: VecDeque<Value>,
    seq: u64,
    next_id: u64,
}

thread_local! {
    static STATE: RefCell<State> = const { RefCell::new(State {
        control: Control::Running,
        sessions: BTreeMap::new(),
        leases: BTreeMap::new(),
        approvals: BTreeMap::new(),
        handoffs: BTreeMap::new(),
        locks: BTreeMap::new(),
        activity: VecDeque::new(),
        seq: 0,
        next_id: 0,
    }) };
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    STATE.with(|state| f(&mut state.borrow_mut()))
}

/// An agent that told the browser it is working, shown by the toolbar pill
/// until it disconnects. Long-lived connections (MCP, stdio, a pipe client)
/// are present for their lifetime; one-shot `agentgo call` commands are not,
/// so an agent using those connects and disconnects explicitly.
struct Presence {
    since: u64,
    last: u64,
    /// The session whose lifetime this presence follows, if it is automatic.
    session: Option<String>,
}

/// One-shot command-line calls: no automatic presence.
const ONE_SHOT_CLIENT: &str = "agentgo-cli";
/// An explicit connection nobody used for this long counts as gone.
const PRESENCE_IDLE_SECS: u64 = 30 * 60;

thread_local! {
    static PRESENCE: RefCell<BTreeMap<String, Presence>> = const { RefCell::new(BTreeMap::new()) };
}

/// `agent.connect {name}`: the agent is working with the browser from now on.
pub fn connect(name: &str, session: Option<&str>) {
    let name: String = name.trim().chars().take(64).collect();
    if name.is_empty() {
        return;
    }
    PRESENCE.with(|presence| {
        let mut presence = presence.borrow_mut();
        let entry = presence.entry(name.clone()).or_insert(Presence {
            since: now(),
            last: now(),
            session: None,
        });
        entry.last = now();
        if session.is_some() {
            entry.session = session.map(str::to_string);
        }
    });
    emit("agent.present", json!({"agent": name}));
    super::state::notify_shell_agent();
}

/// `agent.disconnect {name}`: the agent finished.
pub fn disconnect(name: &str) -> bool {
    let removed = PRESENCE.with(|presence| presence.borrow_mut().remove(name.trim()).is_some());
    if removed {
        emit("agent.absent", json!({"agent": name}));
        super::state::notify_shell_agent();
    }
    removed
}

/// Connected agents, oldest first; idle explicit connections expire.
fn present_agents() -> Vec<Value> {
    PRESENCE.with(|presence| {
        let mut presence = presence.borrow_mut();
        let cutoff = now().saturating_sub(PRESENCE_IDLE_SECS);
        presence.retain(|_, entry| entry.session.is_some() || entry.last >= cutoff);
        presence
            .iter()
            .map(|(name, entry)| json!({"agent": name, "since": entry.since, "last": entry.last}))
            .collect()
    })
}

/// Whether any agent is connected right now (what the chrome's pill shows).
pub fn agent_present() -> bool {
    !present_agents().is_empty()
}

/// Registers a connected agent session.
pub fn register(id: &str, client: &str, transport: &'static str, out: Sender<String>) {
    with(|state| {
        state.sessions.insert(
            id.to_string(),
            Session {
                client: client.chars().take(64).collect(),
                transport,
                out,
                subscribed: false,
                connected_at: now(),
                actions: 0,
                last: String::new(),
                current: None,
            },
        );
    });
    emit("agent.connected", json!({"session": id, "client": client}));
    if client != ONE_SHOT_CLIENT {
        connect(if client.trim().is_empty() { id } else { client }, Some(id));
    }
}

/// Removes a session and releases its leases. Tabs stay open for the human.
pub fn unregister(id: &str) {
    let existed = with(|state| {
        // Locked tabs stay reserved for the next session of the same client.
        for (tab, holder) in state.leases.iter_mut() {
            if holder == id {
                if let Some(client) = state.locks.get(tab) {
                    *holder = format!("locked:{client}");
                }
            }
        }
        state.leases.retain(|_, holder| holder != id);
        state.approvals.retain(|_, approval| approval.session != id);
        state.sessions.remove(id).is_some()
    });
    if existed {
        emit("agent.disconnected", json!({"session": id}));
    }
    let gone: Vec<String> = PRESENCE.with(|presence| {
        presence
            .borrow()
            .iter()
            .filter(|(_, entry)| entry.session.as_deref() == Some(id))
            .map(|(name, _)| name.clone())
            .collect()
    });
    for name in gone {
        disconnect(&name);
    }
}

/// Sends one already-encoded line to a session's transport.
pub fn send_line(session: &str, line: String) {
    with(|state| {
        if let Some(entry) = state.sessions.get(session) {
            let _ = entry.out.send(line);
        }
    });
}

/// Enables event delivery for a session.
pub fn subscribe(session: &str, on: bool) {
    with(|state| {
        if let Some(entry) = state.sessions.get_mut(session) {
            entry.subscribed = on;
        }
    });
}

/// Broadcasts an event to subscribed sessions and refreshes the shell's
/// agent indicator. Events carry a monotonic sequence number.
pub fn emit(event: &str, params: Value) {
    let lines: Vec<(Sender<String>, String)> = with(|state| {
        state.seq += 1;
        let line = json!({"v": 1, "event": event, "seq": state.seq, "params": params}).to_string();
        state
            .sessions
            .values()
            .filter(|session| session.subscribed)
            .map(|session| (session.out.clone(), format!("{line}\n")))
            .collect()
    });
    for (out, line) in lines {
        let _ = out.send(line);
    }
    super::state::notify_shell_agent();
}

/// Records one agent action for the operator's activity view and audit log.
pub fn record(caller: &Caller, method: &str, summary: &str, ok: bool) {
    if caller.is_shell() {
        return;
    }
    let entry = json!({
        "at": now(), "session": caller.session, "method": method,
        "summary": summary.chars().take(140).collect::<String>(), "ok": ok,
    });
    with(|state| {
        if let Some(session) = state.sessions.get_mut(&caller.session) {
            session.actions += 1;
            session.last = method.to_string();
        }
        if state.activity.len() >= 200 {
            state.activity.pop_front();
        }
        state.activity.push_back(entry.clone());
    });
    // Any agent activity keeps the explicit connections alive.
    PRESENCE.with(|presence| {
        for entry in presence.borrow_mut().values_mut() {
            entry.last = now();
        }
    });
    super::state::audit(&entry);
    super::state::notify_shell_agent();
}

/// Checks that the caller may use a tab, leasing unowned tabs to the first
/// agent that touches them. The human operator is never blocked.
pub fn authorize_tab(caller: &Caller, tab_id: &str) -> Result<(), Fail> {
    if caller.is_shell() {
        return Ok(());
    }
    super::profiles::agent_may_use(tab_id)?;
    with(|state| {
        let client = state
            .sessions
            .get(&caller.session)
            .map(|s| s.client.clone())
            .unwrap_or_default();
        if state
            .leases
            .get(tab_id)
            .is_some_and(|holder| *holder == format!("locked:{client}"))
        {
            state
                .leases
                .insert(tab_id.to_string(), caller.session.clone());
        }
        match state.leases.get(tab_id) {
            Some(holder) if holder != &caller.session => {
                return Err(Fail::new(
                    ErrorCode::Unauthorized,
                    format!("tab {tab_id} is leased to another agent session; create your own tab with tabs.create"),
                ))
            }
            Some(_) => {}
            None => {
                state.leases.insert(tab_id.to_string(), caller.session.clone());
            }
        }
        if let Some(session) = state.sessions.get_mut(&caller.session) {
            session.current = Some(tab_id.to_string());
        }
        Ok(())
    })
}

/// `agent.lock`: keeps a tab reserved for this client name when the session
/// disconnects; the next session of that client gets it back. `agent.unlock`
/// ends that.
pub fn set_lock(caller: &Caller, tab_id: &str, on: bool) -> Result<(), Fail> {
    authorize_tab(caller, tab_id)?;
    with(|state| {
        let client = state
            .sessions
            .get(&caller.session)
            .map(|s| s.client.clone())
            .unwrap_or_else(|| caller.session.clone());
        if on {
            state.locks.insert(tab_id.to_string(), client);
        } else {
            state.locks.remove(tab_id);
        }
    });
    Ok(())
}

/// `browser.detach`: the session lets go of its tabs (they stay open).
pub fn release_all(session: &str) -> Vec<String> {
    with(|state| {
        let tabs: Vec<String> = state
            .leases
            .iter()
            .filter(|(_, holder)| *holder == session)
            .map(|(tab, _)| tab.clone())
            .collect();
        state.leases.retain(|_, holder| holder != session);
        for tab in &tabs {
            state.locks.remove(tab);
        }
        if let Some(entry) = state.sessions.get_mut(session) {
            entry.current = None;
        }
        tabs
    })
}

/// The tab a session addresses when it names none: the one it used last.
/// Agents never default to whatever tab another agent or the human is on.
pub fn default_tab(caller: &Caller) -> Option<String> {
    with(|state| {
        let session = state.sessions.get(&caller.session)?;
        session
            .current
            .clone()
            .filter(|tab| state.leases.get(tab) == Some(&caller.session))
            .or_else(|| {
                state
                    .leases
                    .iter()
                    .find(|(_, holder)| *holder == &caller.session)
                    .map(|(tab, _)| tab.clone())
            })
    })
}

/// Whether a tab is visible to the caller (own or unleased).
pub fn can_see_tab(caller: &Caller, tab_id: &str) -> bool {
    caller.is_shell()
        || with(|state| {
            state
                .leases
                .get(tab_id)
                .is_none_or(|holder| holder == &caller.session)
        })
}

/// Session id that currently drives a tab, if any.
pub fn lease_holder(tab_id: &str) -> Option<String> {
    with(|state| state.leases.get(tab_id).cloned())
}

/// Drops state tied to a closed tab.
pub fn forget_tab(tab_id: &str) {
    with(|state| {
        state.leases.remove(tab_id);
        state.handoffs.remove(tab_id);
        state.locks.remove(tab_id);
        state
            .approvals
            .retain(|_, approval| approval.tab_id != tab_id);
    });
    super::accounts::forget_filled(tab_id);
}

/// Current operator control state.
pub fn control() -> Control {
    with(|state| state.control)
}

/// Sets operator control. Only the trusted shell reaches this.
pub fn set_control(control: Control) {
    with(|state| {
        state.control = control;
        if control == Control::Stopped {
            // Stop revokes everything an agent was waiting on.
            state.approvals.clear();
        }
    });
    emit("agent.control", json!({"state": control.as_str()}));
}

/// Blocks an agent action while paused and rejects it while stopped.
pub async fn gate(caller: &Caller) -> Result<(), Fail> {
    if caller.is_shell() {
        return Ok(());
    }
    for _ in 0..6_000 {
        match control() {
            Control::Running => return Ok(()),
            Control::Stopped => {
                return Err(Fail::new(
                    ErrorCode::Unauthorized,
                    "agent control was stopped by the operator; it must be re-enabled in AgentGo before any further action",
                ))
            }
            Control::Paused => exec::sleep(100).await,
        }
    }
    Err(Fail::new(
        ErrorCode::Timeout,
        "agent control stayed paused for 10 minutes",
    ))
}

/// Requires an operator approval bound to this exact tab, origin and target.
/// A granted approval is consumed by the action that uses it.
pub fn require_approval(
    caller: &Caller,
    tab_id: &str,
    origin: &str,
    target: &str,
    reason: &str,
    supplied: Option<&str>,
) -> Result<(), Fail> {
    if caller.is_shell() {
        return Ok(());
    }
    let outcome = with(|state| {
        if let Some(id) = supplied {
            match state.approvals.get(id) {
                Some(approval)
                    if approval.status == "granted"
                        && now().saturating_sub(approval.decided) <= APPROVAL_SECONDS
                        && approval.session == caller.session
                        && approval.tab_id == tab_id
                        && approval.origin == origin
                        && approval.target == target =>
                {
                    state.approvals.remove(id);
                    return Ok(());
                }
                Some(approval) if approval.status == "denied" => {
                    state.approvals.remove(id);
                    return Err(Fail::new(
                        ErrorCode::PolicyDenied,
                        "the operator denied this action",
                    ));
                }
                Some(approval) if approval.status == "pending" => {
                    return Err(Fail::new(
                        ErrorCode::ApprovalRequired,
                        "approval is still pending; call approval.wait",
                    )
                    .with(json!({"approvalId": id})));
                }
                _ => {} // changed target/origin invalidates the approval
            }
        }
        // Expired requests are dropped; an agent cannot flood the person.
        let at = now();
        state.approvals.retain(|_, approval| match approval.status {
            "pending" => at.saturating_sub(approval.created) <= APPROVAL_SECONDS,
            "granted" => at.saturating_sub(approval.decided) <= APPROVAL_SECONDS,
            _ => true,
        });
        let pending = state
            .approvals
            .values()
            .filter(|approval| approval.session == caller.session && approval.status == "pending")
            .count();
        if pending >= MAX_PENDING {
            return Err(Fail::new(
                ErrorCode::PolicyDenied,
                format!("{MAX_PENDING} approvals are already waiting for the person; wait for them first"),
            ));
        }
        state.next_id += 1;
        let id = format!("ap-{}", state.next_id);
        state.approvals.insert(
            id.clone(),
            Approval {
                session: caller.session.clone(),
                tab_id: tab_id.to_string(),
                origin: origin.to_string(),
                target: target.to_string(),
                reason: reason.to_string(),
                status: "pending",
                created: now(),
                decided: 0,
            },
        );
        Err(Fail::new(
            ErrorCode::ApprovalRequired,
            format!("operator approval required: {reason}. Call approval.wait, then retry with approvalId"),
        )
        .with(json!({"approvalId": id, "target": target, "origin": origin})))
    });
    if outcome.is_err() {
        emit(
            "approval.requested",
            json!({"tabId": tab_id, "reason": reason}),
        );
        super::state::present_agent_panel();
    }
    outcome
}

/// Operator decision on a pending approval.
pub fn decide_approval(id: &str, grant: bool) -> Result<(), Fail> {
    with(|state| match state.approvals.get_mut(id) {
        Some(approval) if approval.status == "pending" => {
            approval.status = if grant { "granted" } else { "denied" };
            approval.decided = now();
            Ok(())
        }
        _ => Err(Fail::params(format!("no pending approval {id}"))),
    })?;
    emit(
        "approval.decided",
        json!({"approvalId": id, "granted": grant}),
    );
    Ok(())
}

/// Status of an approval as seen by the requesting session.
pub fn approval_status(caller: &Caller, id: &str) -> Option<&'static str> {
    with(|state| {
        state
            .approvals
            .get(id)
            .filter(|approval| approval.session == caller.session)
            // Approvals expire after five minutes without a decision.
            .map(|approval| {
                if (approval.status == "pending"
                    && now().saturating_sub(approval.created) > APPROVAL_SECONDS)
                    || (approval.status == "granted"
                        && now().saturating_sub(approval.decided) > APPROVAL_SECONDS)
                {
                    "expired"
                } else {
                    approval.status
                }
            })
    })
}

/// Marks a tab as waiting for the human (CAPTCHA, login, 2FA). Agent actions
/// on the tab are refused until the operator resumes it.
pub fn begin_handoff(tab_id: &str, reason: &str) -> String {
    let (id, fresh) = with(|state| {
        if let Some((id, _)) = state.handoffs.get(tab_id) {
            return (id.clone(), false);
        }
        state.next_id += 1;
        let id = format!("ho-{}", state.next_id);
        state
            .handoffs
            .insert(tab_id.to_string(), (id.clone(), reason.to_string()));
        (id, true)
    });
    if fresh {
        emit(
            "handoff.requested",
            json!({"tabId": tab_id, "handoffId": id, "reason": reason}),
        );
        super::state::notify_operator(&format!("AgentGo needs you: {reason}"));
        super::state::present_agent_panel();
    }
    id
}

/// A tab that waits for the person, if any (the oldest request first).
pub fn waiting_tab() -> Option<String> {
    with(|state| {
        state
            .handoffs
            .iter()
            .min_by(|a, b| a.1 .0.cmp(&b.1 .0))
            .map(|(tab, _)| tab.clone())
    })
}

/// Active handoff for a tab.
pub fn handoff(tab_id: &str) -> Option<(String, String)> {
    with(|state| state.handoffs.get(tab_id).cloned())
}

/// Clears a handoff after the human finished and the page was re-verified.
pub fn end_handoff(tab_id: &str) {
    if with(|state| state.handoffs.remove(tab_id)).is_some() {
        emit("handoff.resolved", json!({"tabId": tab_id}));
    }
}

/// The agent gave up on a page that waits for the person (it navigated
/// away): the person is no longer asked, and the agent panel says so.
pub fn withdraw_handoff(tab_id: &str) {
    if with(|state| state.handoffs.remove(tab_id)).is_some() {
        emit("handoff.withdrawn", json!({"tabId": tab_id}));
        super::state::log_event(&format!("AG-RUNTIME handoff-withdrawn tab={tab_id}"));
        super::state::notify_shell_agent();
    }
}

/// Refuses agent actions on a tab that is waiting for the human.
pub fn check_handoff(caller: &Caller, tab_id: &str) -> Result<(), Fail> {
    if caller.is_shell() {
        return Ok(());
    }
    match handoff(tab_id) {
        Some((id, reason)) => Err(Fail::new(
            ErrorCode::HumanHandoff,
            format!("waiting for the human operator: {reason}. Call handoff.wait, or navigate this tab elsewhere to give up on the page; do not attempt to solve it"),
        )
        .with(json!({"handoffId": id, "tabId": tab_id}))),
        None => Ok(()),
    }
}

/// Snapshot for the operator's agent panel and `agent.status`.
pub fn status() -> Value {
    let out = with(|state| {
        json!({
            "control": state.control.as_str(),
            "sessions": state.sessions.iter().map(|(id, session)| json!({
                "session": id,
                "client": session.client,
                "transport": session.transport,
                "connectedAt": session.connected_at,
                "actions": session.actions,
                "last": session.last,
                "tabs": state.leases.iter().filter(|(_, holder)| *holder == id).map(|(tab, _)| tab).collect::<Vec<_>>(),
                "locked": state.locks.iter().filter(|(tab, _)| state.leases.get(*tab) == Some(id)).map(|(tab, _)| tab).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
            "approvals": state.approvals.iter().filter(|(_, approval)| approval.status == "pending").map(|(id, approval)| json!({
                "approvalId": id,
                "session": approval.session,
                "tabId": approval.tab_id,
                "origin": approval.origin,
                "target": approval.target,
                "reason": approval.reason,
            })).collect::<Vec<_>>(),
            "handoffs": state.handoffs.iter().map(|(tab, (id, reason))| json!({
                "handoffId": id, "tabId": tab, "reason": reason,
            })).collect::<Vec<_>>(),
            "activity": state.activity.iter().rev().take(40).collect::<Vec<_>>(),
        })
    });
    let mut out = out;
    out["agents"] = json!(present_agents());
    out
}

/// Number of connected agent sessions.
pub fn session_count() -> usize {
    with(|state| state.sessions.len())
}

/// The client name a session gave in `session.hello` ("agent" if none).
pub fn client_of(session: &str) -> String {
    with(|state| state.sessions.get(session).map(|s| s.client.clone()))
        .filter(|client| !client.is_empty() && client != ONE_SHOT_CLIENT)
        .unwrap_or_else(|| "agent".to_string())
}
