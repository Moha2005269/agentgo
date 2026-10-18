//! Shared runtime state: run options and UI-thread-only handles.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicI32;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Mutex, OnceLock};
use std::thread;

use agentgo_protocol::{ErrorCode, FrameConfig, FrameDecoder, Request, Response};
use agentgo_session::InstanceId;
use agentgo_storage::{AgentGoPaths, AtomicStateStore, ProfileCatalog, SqliteStateStore};
use agentgo_storage::{InstanceEndpointRecord, InstanceEndpointStore};
use cef::*;

const DEFAULT_PROFILE_ID: &str = "default";
const LANDING_URL: &str = "agentgo://shell/index.html?embedded=1";

fn default_settings() -> serde_json::Value {
    serde_json::json!({
        "theme": "dark",
        "accent": "#FF6A2B",
        "searchEngine": "Google",
        "motion": true,
        "hardwareAcceleration": true,
        "captchaNotifications": true,
        "downloadsEnabled": false,
        "adBlock": true,
        "fingerprintProtection": true,
        "dangerousSiteProtection": true,
        "autoUpdate": true,
        "blockThirdPartyCookies": true,
        "stripTracking": true,
        "vpn": false,
        "nativeControl": false,
        "controlMode": "approval",
        "agentTabs": "auto",
        "profileId": DEFAULT_PROFILE_ID
    })
}

/// Reports the selected compositor without exposing raw process arguments.
/// Hardware acceleration is the desktop default. Software modes are explicit
/// diagnostics escape hatches for systems with unusual graphics drivers.
pub fn render_mode() -> &'static str {
    match std::env::var("AGENTGO_RENDER_MODE").as_deref() {
        Ok("software") => "software-fallback",
        Ok("swiftshader") => "swiftshader",
        _ => "hardware",
    }
}

/// Held for the life of the process: exclusive lock on the browser's
/// Chromium root so two instances can never share (and corrupt) a profile.
static ROOT_LOCK: OnceLock<std::fs::File> = OnceLock::new();
/// Set when this run's Chromium root is throwaway and is removed at exit.
static EPHEMERAL: AtomicBool = AtomicBool::new(false);
/// Set once CEF can accept posted tasks.
static CEF_READY: AtomicBool = AtomicBool::new(false);

pub(super) fn lock_exclusive(path: &std::path::Path) -> Option<std::fs::File> {
    use std::os::windows::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .share_mode(0)
        .open(path)
        .ok()
}

fn runs_dir() -> PathBuf {
    std::env::temp_dir().join("agentgo-runs")
}

/// Deletes throwaway Chromium roots left by runs that are no longer alive.
/// A live run holds an exclusive lock on `<root>.lock` next to its directory
/// (outside it, so the run can delete its own directory at exit).
fn sweep_stale_runs() {
    let Ok(entries) = std::fs::read_dir(runs_dir()) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let lock_path = path.with_extension("lock");
            if let Some(lock) = lock_exclusive(&lock_path) {
                let _ = std::fs::remove_dir_all(&path);
                drop(lock);
                let _ = std::fs::remove_file(&lock_path);
            }
        } else if path.extension().is_some_and(|ext| ext == "lock")
            && !path.with_extension("").exists()
        {
            // Lock of a run that already removed its directory.
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// Chooses the Chromium root for this run before CEF starts.
///
/// A visible launch, or an agent launch that names a profile, uses the
/// durable per-profile root so cookies and logins survive restarts. Agent
/// launches without a profile, smoke runs, `--ephemeral`, and any launch that
/// finds the profile already open get an isolated throwaway root instead.
pub fn prepare_profile_root(options: &mut RunOptions) {
    sweep_stale_runs();
    let (paths, settings_path, _) = persistent_paths(&options.cache_root);
    let wants_durable = !options.ephemeral
        && !options.smoke
        && (!options.stdio || !options.profile_id.trim().is_empty());
    if wants_durable {
        let profile = if options.profile_id.trim().is_empty() {
            load_settings(&settings_path)
                .get("profileId")
                .and_then(|value| value.as_str())
                .unwrap_or(DEFAULT_PROFILE_ID)
                .to_string()
        } else {
            options.profile_id.trim().to_string()
        };
        let root = paths.profile_dir(&profile).join("cef");
        if std::fs::create_dir_all(&root).is_ok() {
            if let Some(lock) = lock_exclusive(&root.join("agentgo.lock")) {
                let _ = ROOT_LOCK.set(lock);
                options.cache_root = root;
                return;
            }
        }
    }
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let root = runs_dir().join(format!("{}-{nonce}", std::process::id()));
    let _ = std::fs::create_dir_all(&root);
    if let Some(lock) = lock_exclusive(&root.with_extension("lock")) {
        let _ = ROOT_LOCK.set(lock);
    }
    options.cache_root = root;
    EPHEMERAL.store(true, Ordering::SeqCst);
}

/// Tries to remove a throwaway Chromium root after CEF has shut down.
/// Chromium usually still holds its singleton lock inside the directory until
/// this process is gone, in which case the next AgentGo launch sweeps it
/// (`sweep_stale_runs`). Nothing outside `%TEMP%\agentgo-runs` is touched.
pub fn remove_ephemeral_root() {
    if EPHEMERAL.load(Ordering::SeqCst) {
        let root = state().options.cache_root.clone();
        if root.starts_with(runs_dir()) {
            let _ = std::fs::remove_dir_all(&root);
        }
    }
}

/// Marks CEF ready and starts the authenticated named-pipe transport.
pub fn start_transports() {
    CEF_READY.store(true, Ordering::SeqCst);
    let instance = &state().instance_id;
    match super::pipe::start(instance, &state().paths.instance_dir(instance)) {
        Ok(()) => log_event(&format!(
            "AG-RUNTIME pipe-listening {}",
            super::pipe::pipe_name(instance)
        )),
        Err(error) => log_event(&format!("AG-RUNTIME pipe-start-failed {error}")),
    }
}

/// Wakes the UI thread to run posted transport work. Callable from any
/// thread.
pub fn wake_ui() {
    wake_message_pump();
    let external = state().options.stdio || state().options.smoke;
    if !external && CEF_READY.load(Ordering::SeqCst) {
        super::app::kick_ui();
    }
}

fn persistent_paths(fallback: &std::path::Path) -> (AgentGoPaths, PathBuf, PathBuf) {
    let paths = std::env::var_os("AGENTGO_DATA_ROOT")
        .map(AgentGoPaths::with_root)
        .or_else(|| AgentGoPaths::system_default().ok())
        .unwrap_or_else(|| AgentGoPaths::with_root(fallback.join("agentgo-data")));
    let settings_path = paths.config_dir().join("settings.json");
    let history_path = paths.data_dir().join("state.db");
    (paths, settings_path, history_path)
}

/// AgentGo's durable data folder (also for throwaway agent runs).
pub fn data_dir() -> PathBuf {
    persistent_paths(&state().options.cache_root).0.data_dir()
}

/// Feature flags saved for the next start (read before CEF starts).
pub fn saved_feature_flags() -> Option<serde_json::Value> {
    let (paths, _, _) = persistent_paths(&std::env::temp_dir());
    std::fs::read(paths.data_dir().join("feature-flags.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
}

fn load_settings(path: &std::path::Path) -> serde_json::Value {
    let Ok(store) = AtomicStateStore::new(path, vec![]) else {
        return default_settings();
    };
    match store.load::<serde_json::Value>() {
        Ok(Some(value)) if value.is_object() => value,
        _ => default_settings(),
    }
}

fn load_history(path: &std::path::Path) -> Vec<serde_json::Value> {
    SqliteStateStore::open(path)
        .and_then(|store| store.load_history(100))
        .unwrap_or_default()
}

/// Options for one browser run.
#[derive(Debug, Clone)]
pub struct RunOptions {
    /// Initial URL for the untrusted content surface. Empty means the
    /// Google home page.
    pub content_url: String,
    /// Run the smoke scenario and exit with a report (CI evidence run).
    pub smoke: bool,
    /// Root of the bundled shell UI (contains index.html).
    pub ui_root: PathBuf,
    /// CEF root cache for the trusted shell/default request context. The
    /// untrusted content context is separate and in-memory in milestone 1.
    pub cache_root: PathBuf,
    /// Log file for structured run evidence.
    pub log_path: PathBuf,
    /// Accept agent requests from stdin and emit responses on stdout.
    pub stdio: bool,
    /// Profile id to open for this run. Empty means the persisted active
    /// profile or the built-in default profile.
    pub profile_id: String,
    /// Operator launch grant: allow downloads for this run only.
    pub allow_downloads: bool,
    /// Operator launch grant: allow `runtime.evaluate` in approval mode.
    pub allow_evaluate: bool,
    /// Operator launch choice of control mode (`approval` or `full`).
    pub control_mode: Option<String>,
    /// Directories an agent may upload files from.
    pub upload_dirs: Vec<PathBuf>,
    /// Use a throwaway profile even when a persistent one is available.
    pub ephemeral: bool,
    /// Open the window in front and activate it even when an agent started
    /// the browser. By default an agent-started window opens behind the
    /// person's own windows and never takes the keyboard focus.
    pub foreground: bool,
    /// Reopen the pages of the last session when no page is named. A launch
    /// by the person does that anyway.
    pub restore: bool,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            content_url: String::new(),
            smoke: false,
            ui_root: PathBuf::from("ui"),
            cache_root: std::env::temp_dir().join("agentgo-spike-cache"),
            log_path: std::env::temp_dir().join("agentgo-spike.log"),
            stdio: false,
            profile_id: String::new(),
            allow_downloads: false,
            allow_evaluate: false,
            control_mode: None,
            upload_dirs: Vec::new(),
            ephemeral: false,
            foreground: false,
            restore: false,
        }
    }
}

impl RunOptions {
    /// Whether the window opens without taking focus. A browser the person
    /// launched themselves (no agent transport) always opens in front, as any
    /// application does; an agent-started one stays out of their way.
    pub fn background(&self) -> bool {
        // AGENTGO_BACKGROUND: an instance another agent started (browser.launch).
        (self.stdio || self.smoke || std::env::var_os("AGENTGO_BACKGROUND").is_some())
            && !self.foreground
    }

    /// Resolved content start URL (Google default per ADR-0006).
    pub fn content_start_url(&self) -> String {
        if self.content_url.is_empty() {
            if self.stdio || self.smoke {
                "https://www.google.com/".to_string()
            } else {
                // The visible desktop launch opens the approved AgentGo
                // landing surface. Agents launched over stdio start on
                // Google so the first command is immediately web-oriented.
                LANDING_URL.to_string()
            }
        } else {
            self.content_url.clone()
        }
    }
}

/// A handle that must only be used on the CEF UI thread. Wrapping makes
/// the state shareable while the invariant stays explicit and asserted.
pub struct UiHandle<T>(pub T);

// SAFETY: CEF objects are affine to the UI thread; every access goes
// through functions that run on that thread (CEF's main UI thread with the
// built-in desktop loop or the bounded external test/stdio pump). The handle
// itself never dereferences from other threads.
unsafe impl<T> Send for UiHandle<T> {}
unsafe impl<T> Sync for UiHandle<T> {}

/// A browser view and the stable protocol id assigned to it. The view stays
/// behind the UI-thread handle; only its opaque id and safe metadata cross
/// the protocol boundary.
pub struct ContentTab {
    /// Stable id exposed to the agent.
    pub id: String,
    /// Native CEF view. Access is restricted to the browser/UI thread.
    pub view: UiHandle<cef::BrowserView>,
    /// CEF browser id, populated when the browser callback fires.
    pub browser_id: Option<i32>,
    /// Whether the renderer for this tab has terminated unexpectedly.
    pub crashed: bool,
    /// Whether CEF currently considers this renderer unresponsive.
    pub unresponsive: bool,
    /// Redacted metadata for the most recent renderer termination.
    pub last_crash: Option<serde_json::Value>,
    /// Document title reported by the renderer.
    pub title: String,
    /// Favicon URL reported by the renderer.
    pub favicon: Option<String>,
    /// Most recent main-frame load error, cleared when read.
    pub load_error: Option<String>,
}

/// Smoke scenario steps (spike evidence state machine).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmokeStep {
    /// Waiting for the trusted shell to finish loading.
    AwaitShell,
    /// Shell loaded; waiting for the initial content navigation (Google).
    AwaitContent,
    /// Content loaded; driving the trusted-origin bridge command.
    AwaitBridgeResult,
    /// Bridge navigation landed; shutting down cleanly.
    Done,
}

/// Global runtime state (browser process only).
pub struct AppState {
    /// Run options snapshot.
    pub options: RunOptions,
    /// Evidence events (ordered).
    pub events: Mutex<Vec<String>>,
    /// Process exit code for the smoke run.
    pub exit_code: AtomicI32,
    /// Set when a clean shutdown was requested.
    pub quit_requested: AtomicBool,
    /// Smoke state machine (see `advance_smoke`).
    pub smoke_step: Mutex<SmokeStep>,
    /// Handles to browser views (UI thread only).
    pub content_view: Mutex<Option<UiHandle<cef::BrowserView>>>,
    /// All content tabs in creation order. The first tab is content-1.
    pub content_tabs: Mutex<Vec<ContentTab>>,
    /// Active content tab id.
    pub active_tab_id: Mutex<String>,
    /// Shared isolated request context for content tabs.
    pub content_context: Mutex<Option<UiHandle<cef::RequestContext>>>,
    /// Monotonic tab id allocator for this browser instance.
    pub next_tab_id: AtomicI32,
    /// Shell view handle (UI thread only).
    pub shell_view: Mutex<Option<UiHandle<cef::BrowserView>>>,
    /// Dedicated trusted browser-level utility surface. It is overlaid only
    /// across the actual drawer/popover rectangle, never over the whole page.
    pub tool_view: Mutex<Option<UiHandle<cef::BrowserView>>>,
    /// Native controller for the bounded utility surface.
    pub tool_overlay: Mutex<Option<UiHandle<cef::OverlayController>>>,
    /// Currently open browser utility (`history`, `downloads`, `settings`).
    pub active_tool: Mutex<Option<String>>,
    /// Top-level composition window (UI thread only).
    pub window: Mutex<Option<UiHandle<cef::Window>>>,
    /// The area below the chrome. Every tab is a full-size child of it.
    pub tab_host: Mutex<Option<UiHandle<cef::Panel>>>,
    /// Profile selected for this instance. Credentials and cookies remain in
    /// CEF's profile context and are never returned here.
    pub active_profile_id: Mutex<String>,
    /// Opaque id for this browser process.
    pub instance_id: String,
    /// Durable non-secret endpoint lifecycle record.
    pub endpoint_store: Option<InstanceEndpointStore>,
    /// Canonical local data layout for this run.
    pub paths: AgentGoPaths,
    /// Whether this run uses a throwaway profile (reported honestly).
    pub ephemeral_profile: AtomicBool,
    /// Last requested/current content URL.
    pub content_url: Mutex<String>,
    /// Whether the content surface is waiting for a navigation to settle.
    pub content_loading: AtomicBool,
    /// Set as soon as an agent or smoke step queues a navigation. This keeps
    /// the shell's automatic first navigation from overwriting an explicit
    /// request that arrived during startup.
    pub navigation_requested: AtomicBool,
    /// External-pump wakeup hint. CEF calls the schedule callback from any
    /// thread; the browser loop consumes this hint on the application thread
    /// before the next non-blocking `do_message_loop_work` call.
    pump_requested: AtomicBool,
    /// Persisted, non-secret local settings.
    pub settings: Mutex<serde_json::Value>,
    /// Settings file used by this run.
    pub settings_path: PathBuf,
    /// SQLite state database used by this run.
    pub history_path: PathBuf,
    /// Bounded in-memory navigation history mirror (URLs/titles only; no page
    /// content). Durable records live in the SQLite WAL store.
    pub history: Mutex<Vec<serde_json::Value>>,
    /// Download records for the current profile/session. File bytes remain in
    /// the scoped download directory and are never returned through this API.
    pub downloads: Mutex<Vec<serde_json::Value>>,
    /// Scoped destination directory for downloads in the active profile.
    pub download_dir: Mutex<PathBuf>,
    /// Redacted renderer crash reports retained for diagnostics.
    pub crash_reports: Mutex<Vec<serde_json::Value>>,
    /// Directory for local renderer crash reports.
    pub crash_reports_path: PathBuf,
}

static STATE: OnceLock<AppState> = OnceLock::new();

// CEF supplies DownloadItemCallback only during an update callback. The
// callback is ref-counted, but it is UI-thread-affine, so retain it in a
// thread-local bounded map rather than putting a native handle in shared
// protocol state. Protocol requests are drained on this same CEF UI thread.
thread_local! {
    static DOWNLOAD_CALLBACKS: RefCell<HashMap<u32, DownloadItemCallback>> =
        RefCell::new(HashMap::new());
}

/// Installs the global state (once, before CEF initialize).
pub fn init_state(options: RunOptions) {
    if let Some(parent) = options.log_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    // One run gets one evidence log. Do not accidentally append a previous
    // smoke run and report stale evidence as a current pass.
    // The previous run's log stays as `*.prev.log`, so a problem seen in it
    // can still be read after the next start.
    let _ = std::fs::rename(
        &options.log_path,
        options.log_path.with_extension("prev.log"),
    );
    let _ = std::fs::write(&options.log_path, "");
    let (paths, settings_path, history_path) = persistent_paths(&options.cache_root);
    let _ = paths.ensure_layout();
    if let Ok(catalog) = ProfileCatalog::open(paths.clone()) {
        let _ = catalog.ensure_default();
    }
    let settings = load_settings(&settings_path);
    let requested_profile = if options.profile_id.trim().is_empty() {
        settings
            .get("profileId")
            .and_then(|value| value.as_str())
            .unwrap_or(DEFAULT_PROFILE_ID)
            .to_string()
    } else {
        options.profile_id.trim().to_string()
    };
    let active_profile_id = ProfileCatalog::open(paths.clone())
        .ok()
        .and_then(|catalog| {
            catalog
                .list()
                .ok()
                .and_then(|profiles| profiles.into_iter().find(|p| p.id == requested_profile))
        })
        .map(|profile| profile.id)
        .unwrap_or_else(|| DEFAULT_PROFILE_ID.to_string());
    let instance_id = InstanceId::generate()
        .map(|id| id.to_string())
        .unwrap_or_else(|_| format!("i-{:032x}", unix_now() ^ u64::from(std::process::id())));
    let endpoint_store = InstanceEndpointStore::open(paths.clone(), &instance_id).ok();
    if let Some(store) = &endpoint_store {
        let record = InstanceEndpointRecord {
            instance_id: instance_id.clone(),
            pid: std::process::id(),
            protocol: agentgo_protocol::PROTOCOL_VERSION.to_string(),
            transport: if options.stdio { "stdio" } else { "desktop" }.to_string(),
            auth_mode: if options.stdio {
                "parent-process"
            } else {
                "interactive"
            }
            .to_string(),
            profile_id: active_profile_id.clone(),
            started_at: unix_now(),
            status: "running".to_string(),
            exit_code: None,
            stopped_at: None,
        };
        let _ = store.start(&record);
    }
    super::adblock::start(&settings);
    let history = load_history(&history_path);
    let use_stdio = options.stdio;
    let _ = STATE.set(AppState {
        options,
        events: Mutex::new(Vec::new()),
        exit_code: AtomicI32::new(0),
        quit_requested: AtomicBool::new(false),
        smoke_step: Mutex::new(SmokeStep::AwaitShell),
        content_view: Mutex::new(None),
        content_tabs: Mutex::new(Vec::new()),
        active_tab_id: Mutex::new("content-1".to_string()),
        content_context: Mutex::new(None),
        next_tab_id: AtomicI32::new(2),
        shell_view: Mutex::new(None),
        tool_view: Mutex::new(None),
        tool_overlay: Mutex::new(None),
        active_tool: Mutex::new(None),
        window: Mutex::new(None),
        tab_host: Mutex::new(None),
        active_profile_id: Mutex::new(active_profile_id.clone()),
        instance_id,
        endpoint_store,
        paths: paths.clone(),
        ephemeral_profile: AtomicBool::new(EPHEMERAL.load(Ordering::SeqCst)),
        content_url: Mutex::new("about:blank".to_string()),
        content_loading: AtomicBool::new(false),
        navigation_requested: AtomicBool::new(false),
        pump_requested: AtomicBool::new(true),
        settings: Mutex::new(settings.clone()),
        settings_path,
        history_path,
        history: Mutex::new(history),
        downloads: Mutex::new(Vec::new()),
        download_dir: {
            let dir = paths.download_dir(&active_profile_id);
            let _ = std::fs::create_dir_all(&dir);
            Mutex::new(dir)
        },
        crash_reports: Mutex::new(Vec::new()),
        crash_reports_path: paths.data_dir().join("crashes"),
    });
    // The VPN reads the data folder and logs through the state.
    super::vpn::start(&settings);
    // Install STATE before starting the reader. An agent can write its first
    // request immediately after process creation; starting the thread first
    // creates a race where wake_message_pump() observes an uninitialized
    // OnceLock and panics before CEF has entered its message loop.
    if use_stdio {
        start_stdio_threads();
    }
}

/// Marks the durable endpoint record stopped before CEF is torn down.
pub fn mark_instance_stopped() {
    let exit_code = state().exit_code.load(Ordering::SeqCst);
    if let Some(store) = &state().endpoint_store {
        let _ = store.stop(exit_code, unix_now());
    }
    // The token is only meaningful while the pipe exists.
    let _ = std::fs::remove_file(
        state()
            .paths
            .instance_dir(&state().instance_id)
            .join("token"),
    );
}

/// Starts the local newline-delimited JSON transport. The reader and writer
/// are deliberately separate from CEF: only the browser/UI thread touches
/// browser objects, while stdout remains a clean machine-readable stream.
fn start_stdio_threads() {
    use super::sessions::{self, Caller, Fail};
    const SESSION: &str = "stdio";
    let (line_tx, line_rx) = mpsc::channel::<String>();
    // The process that launched AgentGo over stdio is the one agent session.
    sessions::register(SESSION, "stdio-agent", "stdio", line_tx);
    let reject = |message: String| {
        super::exec::post(move || {
            let reply = Err(Fail::new(ErrorCode::InvalidEnvelope, message));
            sessions::send_line(SESSION, super::dispatch::encode(0, reply));
        });
        wake_message_pump();
    };
    thread::Builder::new()
        .name("agentgo-stdio-reader".into())
        .spawn(move || {
            use std::io::Read;
            let mut input = std::io::stdin();
            let mut decoder = FrameDecoder::new(FrameConfig::default());
            let mut buffer = [0_u8; 8192];
            loop {
                let read = match input.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                if let Err(error) = decoder.push(&buffer[..read]) {
                    reject(error.to_string());
                    continue;
                }
                while let Some(message) = decoder.pop_message::<Request>() {
                    match message {
                        Ok(request) => {
                            super::exec::post(move || {
                                super::dispatch::handle_request(
                                    Caller {
                                        session: SESSION.into(),
                                    },
                                    request,
                                );
                            });
                            wake_ui();
                        }
                        Err(error) => reject(error.to_string()),
                    }
                }
            }
        })
        .expect("spawn AgentGo stdio reader");
    spawn_line_writer("agentgo-stdio-writer", line_rx, std::io::stdout());
}

/// Drains a session's outgoing lines into its transport on a writer thread.
pub fn spawn_line_writer(
    name: &str,
    lines: Receiver<String>,
    mut output: impl std::io::Write + Send + 'static,
) {
    let _ = thread::Builder::new().name(name.into()).spawn(move || {
        while let Ok(line) = lines.recv() {
            if output.write_all(line.as_bytes()).is_err() || output.flush().is_err() {
                break;
            }
        }
    });
}

/// Records CEF's next external-message-pump request. CEF may call this from a
/// non-UI thread, so the function only touches synchronized/atomic state; the
/// actual `do_message_loop_work` call stays on the application thread.
pub fn schedule_message_pump_work(_delay_ms: i64) {
    state().pump_requested.store(true, Ordering::SeqCst);
}

/// Wakes the host loop when local protocol input arrives.
pub fn wake_message_pump() {
    state().pump_requested.store(true, Ordering::SeqCst);
}

/// Takes the current wakeup hint. The host still pumps on a bounded cadence
/// even when false; the hint only lets active work use the shorter cadence.
pub fn take_message_pump_request() -> bool {
    state().pump_requested.swap(false, Ordering::SeqCst)
}

/// The global state; panics when called before `init_state`.
pub fn state() -> &'static AppState {
    STATE
        .get()
        .expect("runtime state must be initialized before use")
}

/// Appends a structured evidence event to the log file and stdout.
/// Thread-safe; used from any CEF thread.
pub fn log_event(event: &str) {
    let line = format!(
        "{} AG-EVENT {event}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    );
    if state().options.stdio {
        eprintln!("{line}");
    } else {
        println!("{line}");
    }
    if let Ok(mut events) = state().events.lock() {
        events.push(line.clone());
    }
    // Append to the evidence file too: a GUI-subsystem process has no
    // usable stdout, so the file is the authoritative record.
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&state().options.log_path)
    {
        let _ = writeln!(f, "{line}");
    }
}

/// Requests a clean shutdown with the given exit code.
pub fn request_quit(code: i32) {
    state()
        .exit_code
        .store(code, std::sync::atomic::Ordering::SeqCst);
    state()
        .quit_requested
        .store(true, std::sync::atomic::Ordering::SeqCst);
    log_event("AG-SMOKE quit-requested");
    cef::quit_message_loop();
}

/// Closes the native window first and lets CEF tear down both browsers before
/// the outer message pump calls `shutdown`. Falls back to immediate shutdown
/// when startup failed before a window existed.
pub fn request_window_close(code: i32) {
    state()
        .exit_code
        .store(code, std::sync::atomic::Ordering::SeqCst);
    let window = state()
        .window
        .lock()
        .ok()
        .and_then(|slot| slot.as_ref().map(|handle| handle.0.clone()));
    if let Some(window) = window {
        super::notify::clear(&window);
        log_event("AG-SMOKE window-close-requested");
        window.close();
    } else {
        request_quit(code);
    }
}

/// Records the top-level window once CEF creates it.
pub fn set_window(window: cef::Window) {
    if let Ok(mut slot) = state().window.lock() {
        *slot = Some(UiHandle(window));
    }
}

/// Records the panel that holds the tabs.
pub fn set_tab_host(host: cef::Panel) {
    if let Ok(mut slot) = state().tab_host.lock() {
        *slot = Some(UiHandle(host));
    }
}

pub fn tab_host() -> Option<cef::Panel> {
    state()
        .tab_host
        .lock()
        .ok()
        .and_then(|slot| slot.as_ref().map(|handle| handle.0.clone()))
}

/// Releases browser/window handles before `cef_shutdown`.
pub fn clear_ui_handles() {
    if let Ok(mut slot) = state().tab_host.lock() {
        *slot = None;
    }
    if let Ok(mut slot) = state().content_view.lock() {
        *slot = None;
    }
    if let Ok(mut tabs) = state().content_tabs.lock() {
        tabs.clear();
    }
    if let Ok(mut active) = state().active_tab_id.lock() {
        *active = "content-1".to_string();
    }
    if let Ok(mut context) = state().content_context.lock() {
        *context = None;
    }
    if let Ok(mut slot) = state().shell_view.lock() {
        *slot = None;
    }
    if let Ok(mut slot) = state().tool_view.lock() {
        *slot = None;
    }
    if let Ok(mut slot) = state().tool_overlay.lock() {
        *slot = None;
    }
    if let Ok(mut active) = state().active_tool.lock() {
        *active = None;
    }
    if let Ok(mut slot) = state().window.lock() {
        *slot = None;
    }
}

/// Registers the isolated content context used by all tabs in this window.
pub fn set_content_context(context: cef::RequestContext) {
    if let Ok(mut slot) = state().content_context.lock() {
        *slot = Some(UiHandle(context));
    }
}

/// Registers a native content tab before its asynchronous browser callbacks
/// begin. Registration is idempotent for the initial tab.
pub fn register_content_tab(id: &str, view: cef::BrowserView) -> Result<(), String> {
    let mut tabs = state()
        .content_tabs
        .lock()
        .map_err(|_| "content tab registry is unavailable".to_string())?;
    if tabs.iter().any(|tab| tab.id == id) {
        return Err(format!("tab {id} already exists"));
    }
    tabs.push(ContentTab {
        id: id.to_string(),
        view: UiHandle(view),
        browser_id: None,
        crashed: false,
        unresponsive: false,
        last_crash: None,
        title: String::new(),
        favicon: None,
        load_error: None,
    });
    Ok(())
}

/// Stores CEF's browser id for a registered tab. The id lets diagnostics and
/// load callbacks associate native events with the stable protocol tab id.
/// The tab a browser id belongs to (safe from any thread).
pub fn tab_for_browser_id(browser_id: i32) -> Option<String> {
    state().content_tabs.lock().ok().and_then(|tabs| {
        tabs.iter()
            .find(|tab| tab.browser_id == Some(browser_id))
            .map(|tab| tab.id.clone())
    })
}

pub fn register_tab_browser_id(tab_id: &str, browser_id: i32) {
    if let Ok(mut tabs) = state().content_tabs.lock() {
        if let Some(tab) = tabs.iter_mut().find(|tab| tab.id == tab_id) {
            tab.browser_id = Some(browser_id);
        }
    }
}

/// Marks the renderer associated with a browser id as unresponsive. The
/// native request handler calls this without touching browser objects from a
/// different thread.
pub fn mark_renderer_unresponsive(browser_id: i32) {
    if let Ok(mut tabs) = state().content_tabs.lock() {
        if let Some(tab) = tabs
            .iter_mut()
            .find(|tab| tab.browser_id == Some(browser_id))
        {
            tab.unresponsive = true;
            log_event(&format!(
                "AG-RUNTIME renderer-unresponsive tab={} browserId={browser_id}",
                tab.id
            ));
        }
    }
}

/// Clears the unresponsive marker after CEF reports that a renderer is
/// responsive again.
pub fn mark_renderer_responsive(browser_id: i32) {
    if let Ok(mut tabs) = state().content_tabs.lock() {
        if let Some(tab) = tabs
            .iter_mut()
            .find(|tab| tab.browser_id == Some(browser_id))
        {
            tab.unresponsive = false;
            log_event(&format!(
                "AG-RUNTIME renderer-responsive tab={} browserId={browser_id}",
                tab.id
            ));
        }
    }
}

/// Records renderer termination as a small local artifact and exposes only
/// redacted metadata through the protocol. Error text is bounded and never
/// includes page HTML, cookies, form values, or request bodies.
pub fn record_renderer_termination(
    browser_id: Option<i32>,
    error_code: i32,
    error_string: Option<&str>,
) {
    let tab_id = browser_id.and_then(|id| {
        state().content_tabs.lock().ok().and_then(|tabs| {
            tabs.iter()
                .find(|tab| tab.browser_id == Some(id))
                .map(|tab| tab.id.clone())
        })
    });
    let error = error_string
        .unwrap_or("renderer process terminated")
        .chars()
        .take(256)
        .collect::<String>();
    let report = serde_json::json!({
        "kind": "renderer_termination",
        "tabId": tab_id,
        "browserId": browser_id,
        "errorCode": error_code,
        "error": error,
        "recordedAt": unix_now()
    });
    if let Ok(mut tabs) = state().content_tabs.lock() {
        if let Some(id) = browser_id {
            if let Some(tab) = tabs.iter_mut().find(|tab| tab.browser_id == Some(id)) {
                tab.crashed = true;
                tab.unresponsive = false;
                tab.last_crash = Some(report.clone());
            }
        }
    }
    let _ = std::fs::create_dir_all(&state().crash_reports_path);
    let report_path = state().crash_reports_path.join(format!(
        "renderer-{}-{}.json",
        unix_now(),
        browser_id.unwrap_or(0)
    ));
    let temp_path = report_path.with_extension("json.tmp");
    if let Ok(bytes) = serde_json::to_vec_pretty(&report) {
        if std::fs::write(&temp_path, bytes).is_ok() {
            let _ = std::fs::rename(&temp_path, &report_path);
        }
    }
    let mut public_report = report;
    if let Some(object) = public_report.as_object_mut() {
        object.insert(
            "path".to_string(),
            serde_json::Value::String(report_path.to_string_lossy().to_string()),
        );
    }
    if let Ok(mut reports) = state().crash_reports.lock() {
        reports.insert(0, public_report.clone());
        reports.truncate(32);
    }
    log_event(&format!(
        "AG-RUNTIME renderer-terminated tab={} browserId={:?} errorCode={error_code}",
        tab_id.as_deref().unwrap_or("unknown"),
        browser_id
    ));
}

/// Returns local crash metadata without reading the report body into the
/// agent response.
pub fn crash_reports_snapshot() -> serde_json::Value {
    let reports = state()
        .crash_reports
        .lock()
        .map(|reports| reports.clone())
        .unwrap_or_default();
    serde_json::json!({"reports":reports,"count":reports.len()})
}

/// Records a native file-dialog request. Returning false from the CEF dialog
/// handler leaves the actual picker with CEF/Windows; this event gives the
/// future native-control lane a safe, auditable handoff point.
pub fn record_file_dialog_request(tab_id: &str, mode: i32, title: Option<&str>) {
    let title = title
        .unwrap_or_default()
        .chars()
        .take(160)
        .collect::<String>();
    log_event(&format!(
        "AG-RUNTIME native-file-dialog-request tab={tab_id} mode={mode} title={title:?}"
    ));
}

/// Returns a fresh monotonic tab id for this browser instance.
pub fn allocate_tab_id() -> String {
    let id = state().next_tab_id.fetch_add(1, Ordering::SeqCst);
    format!("content-{id}")
}

/// Returns the current active tab id.
pub fn active_tab_id() -> String {
    state()
        .active_tab_id
        .lock()
        .map(|id| id.clone())
        .unwrap_or_else(|_| "content-1".to_string())
}

/// Returns the profile selected for this browser instance.
pub fn active_profile_id() -> String {
    state()
        .active_profile_id
        .lock()
        .map(|id| id.clone())
        .unwrap_or_else(|_| DEFAULT_PROFILE_ID.to_string())
}

/// Selects a registered tab and returns a stable snapshot of the selection.
pub fn activate_tab(id: &str) -> Result<(), String> {
    if !tab_exists(id) {
        return Err(format!("unknown tab id {id}"));
    }
    if let Ok(mut active) = state().active_tab_id.lock() {
        *active = id.to_string();
    }
    super::app::relayout_registered_content();
    if let Some(host) = content_browser_host_for_tab(id) {
        focus_page(&host);
    }
    notify_shell_state();
    Ok(())
}

/// Keeps the operating-system title of the window (taskbar tooltip, Alt+Tab,
/// screen readers) in step with the visible tab. AgentGo draws its own title
/// strip, so without this the window would have no name at all.
pub fn refresh_window_title() {
    if cef::currently_on(cef::ThreadId::UI) == 0 {
        return;
    }
    let active = active_tab_id();
    let tab_title = state()
        .content_tabs
        .lock()
        .ok()
        .and_then(|tabs| {
            tabs.iter()
                .find(|tab| tab.id == active)
                .map(|tab| tab.title.clone())
        })
        .unwrap_or_default();
    let tab_title = tab_title.trim();
    let text = if tab_title.is_empty() {
        "AgentGo".to_string()
    } else if tab_title.ends_with("AgentGo") {
        tab_title.to_string()
    } else {
        format!("{tab_title} - AgentGo")
    };
    let Some(window) = state()
        .window
        .lock()
        .ok()
        .and_then(|slot| slot.as_ref().map(|handle| handle.0.clone()))
    else {
        return;
    };
    if cef::CefString::from(&window.title()).to_string() != text {
        window.set_title(Some(&cef::CefString::from(text.as_str())));
    }
}

/// Where a tab that an agent opens goes in the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// It becomes the visible tab.
    Front,
    /// It joins the tab strip without replacing the tab the person is using.
    Background,
}

/// Whether the person is working in this browser right now: one of its windows
/// has the operating system's foreground and a real keyboard or mouse event
/// arrived a moment ago. DevTools input never passes through the operating
/// system's input stream, so an agent acting on a page cannot make this true.
pub fn human_active() -> bool {
    const RECENT_MS: u64 = 4_000;
    foreground_is_ours() && input_idle_ms().is_some_and(|idle| idle < RECENT_MS)
}

#[cfg(target_os = "windows")]
fn foreground_is_ours() -> bool {
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowThreadProcessId,
    };
    // SAFETY: both calls only read system state; `owner` is a valid out value.
    unsafe {
        let foreground = GetForegroundWindow();
        if foreground.is_null() {
            return false;
        }
        let mut owner = 0_u32;
        GetWindowThreadProcessId(foreground, &mut owner);
        owner == GetCurrentProcessId()
    }
}

#[cfg(not(target_os = "windows"))]
fn foreground_is_ours() -> bool {
    window_is_active()
}

fn window_is_active() -> bool {
    state()
        .window
        .lock()
        .ok()
        .and_then(|slot| slot.as_ref().map(|handle| handle.0.is_active() != 0))
        .unwrap_or(false)
}

#[cfg(target_os = "windows")]
fn input_idle_ms() -> Option<u64> {
    use windows_sys::Win32::System::SystemInformation::GetTickCount;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
    let mut info = LASTINPUTINFO {
        cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
        dwTime: 0,
    };
    // SAFETY: `info` is a valid LASTINPUTINFO whose size field is set.
    if unsafe { GetLastInputInfo(&mut info) } == 0 {
        return None;
    }
    // SAFETY: GetTickCount has no preconditions.
    let now = unsafe { GetTickCount() };
    Some(u64::from(now.wrapping_sub(info.dwTime)))
}

#[cfg(not(target_os = "windows"))]
fn input_idle_ms() -> Option<u64> {
    None
}

/// The operator's `agentTabs` choice: `auto` (default), `background`, `follow`.
fn agent_tabs_mode() -> String {
    settings_snapshot()
        .get("agentTabs")
        .and_then(|value| value.as_str())
        .filter(|mode| matches!(*mode, "auto" | "background" | "follow"))
        .unwrap_or("auto")
        .to_string()
}

/// Placement for a tab an agent opens or asks to show. In `auto` the agent
/// follows its own work, unless the person is using the window: their tab is
/// never swapped out from under them.
pub fn agent_tab_placement() -> Placement {
    match agent_tabs_mode().as_str() {
        "background" => Placement::Background,
        "follow" => Placement::Front,
        _ if human_active() => Placement::Background,
        _ => Placement::Front,
    }
}

/// `tabs.activate` for an agent. Honors the person's placement preference
/// and tells the agent plainly when the tab stayed where it was.
pub fn activate_tab_for_agent(id: &str) -> Result<serde_json::Value, String> {
    if !tab_exists(id) {
        return Err(format!("unknown tab id {id}"));
    }
    if active_tab_id() != id && agent_tab_placement() == Placement::Background {
        return Ok(serde_json::json!({
            "accepted": true,
            "applied": false,
            "activeTabId": active_tab_id(),
            "reason": "the person is using this window; the tab stays in the background and keeps working"
        }));
    }
    activate_tab(id)?;
    Ok(serde_json::json!({"accepted": true, "applied": true, "activeTabId": active_tab_id()}))
}

/// Moves the keyboard focus into a page, but only while the person already is
/// in this window. An inactive window is never pulled forward: agents do not
/// need operating-system focus because their input is synthetic.
fn focus_page(host: &BrowserHost) {
    if window_is_active() {
        host.set_focus(1);
    }
}

/// What an agent can tell about the window without being able to change it.
pub fn window_state() -> Result<serde_json::Value, String> {
    let window = state()
        .window
        .lock()
        .ok()
        .and_then(|slot| slot.as_ref().map(|handle| handle.0.clone()))
        .ok_or_else(|| "browser window is not ready".to_string())?;
    let bounds = window.bounds();
    Ok(serde_json::json!({
        "visible": window_is_shown(&window),
        "active": window.is_active() != 0,
        "foreground": window_is_foreground(&window),
        "minimized": window.is_minimized() != 0,
        "maximized": window.is_maximized() != 0,
        "fullscreen": window.is_fullscreen() != 0,
        "presentation": if state().options.background() { "background" } else { "foreground" },
        "humanActive": human_active(),
        "agentTabs": agent_tabs_mode(),
        "bounds": {"x": bounds.x, "y": bounds.y, "width": bounds.width, "height": bounds.height},
        "activeTabId": active_tab_id(),
    }))
}

/// Asks the operating system, not the Views layer: a window that was shown
/// without activation is visible even though Views never saw a `Show()`.
#[cfg(target_os = "windows")]
fn window_is_shown(window: &cef::Window) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::IsWindowVisible;
    let hwnd = window.window_handle().0 as windows_sys::Win32::Foundation::HWND;
    // SAFETY: IsWindowVisible accepts any handle value.
    !hwnd.is_null() && unsafe { IsWindowVisible(hwnd) } != 0
}

#[cfg(not(target_os = "windows"))]
fn window_is_shown(window: &cef::Window) -> bool {
    window.is_visible() != 0
}

#[cfg(target_os = "windows")]
fn window_is_foreground(window: &cef::Window) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
    let hwnd = window.window_handle().0 as windows_sys::Win32::Foundation::HWND;
    // SAFETY: GetForegroundWindow has no preconditions.
    !hwnd.is_null() && unsafe { GetForegroundWindow() } == hwnd
}

#[cfg(not(target_os = "windows"))]
fn window_is_foreground(_window: &cef::Window) -> bool {
    false
}

/// Returns whether a stable tab id is registered.
pub fn tab_exists(id: &str) -> bool {
    state()
        .content_tabs
        .lock()
        .map(|tabs| tabs.iter().any(|tab| tab.id == id))
        .unwrap_or(false)
}

/// Resolves an optional protocol tab id to a registered tab.
pub fn requested_tab_id(params: &serde_json::Value) -> Result<String, String> {
    let id = params
        .get("tabId")
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .unwrap_or_else(active_tab_id);
    if tab_exists(&id) {
        Ok(id)
    } else {
        Err(format!("unknown tab id {id}"))
    }
}

/// Returns a view by stable tab id (UI thread only).
pub fn content_view_for_tab(id: &str) -> Option<cef::BrowserView> {
    state().content_tabs.lock().ok().and_then(|tabs| {
        tabs.iter()
            .find(|tab| tab.id == id)
            .map(|tab| tab.view.0.clone())
    })
}

/// Returns the shared isolated request context (UI thread only).
pub fn content_context() -> Option<cef::RequestContext> {
    state()
        .content_context
        .lock()
        .ok()
        .and_then(|slot| slot.as_ref().map(|handle| handle.0.clone()))
}

/// Puts the tab strip in this order (ids not named keep their place after).
pub fn reorder_tabs(order: &[String]) {
    if let Ok(mut tabs) = state().content_tabs.lock() {
        tabs.sort_by_key(|tab| {
            order
                .iter()
                .position(|id| *id == tab.id)
                .unwrap_or(usize::MAX)
        });
    }
    notify_shell_state();
}

/// Views and ids of the tabs in the main window (not detached into another
/// window), in the same order, for its layout.
pub fn main_window_tabs() -> (Vec<cef::BrowserView>, Vec<String>) {
    state()
        .content_tabs
        .lock()
        .map(|tabs| {
            tabs.iter()
                .filter(|tab| super::winmgr::window_of(&tab.id).is_none())
                .map(|tab| (tab.view.0.clone(), tab.id.clone()))
                .unzip()
        })
        .unwrap_or_default()
}

/// Returns content tab ids in the same stable order as [`content_views`].
/// The compositor uses these ids for visibility instead of comparing CEF
/// wrapper handles, whose identity method is not reliable across clones.
pub fn content_tab_ids() -> Vec<String> {
    state()
        .content_tabs
        .lock()
        .map(|tabs| tabs.iter().map(|tab| tab.id.clone()).collect())
        .unwrap_or_default()
}

/// Removes a tab from the registry and returns its native view. The caller
/// must detach/close the view before dropping the returned handle. Any tab can
/// close while another tab remains.
pub fn remove_content_tab(id: &str) -> Result<(cef::BrowserView, String), String> {
    let mut tabs = state()
        .content_tabs
        .lock()
        .map_err(|_| "content tab registry is unavailable".to_string())?;
    if tabs.len() <= 1 {
        return Err("the last tab cannot be closed".into());
    }
    let index = tabs
        .iter()
        .position(|tab| tab.id == id)
        .ok_or_else(|| format!("unknown tab id {id}"))?;
    let removed = tabs.remove(index);
    let next_active = if active_tab_id() == id {
        tabs.first()
            .map(|tab| tab.id.clone())
            .unwrap_or_else(|| "content-1".to_string())
    } else {
        active_tab_id()
    };
    if let Ok(mut active) = state().active_tab_id.lock() {
        *active = next_active.clone();
    }
    Ok((removed.view.0, next_active))
}

/// The active content view handle (UI thread only).
pub fn content_view() -> Option<cef::BrowserView> {
    content_view_for_tab(&active_tab_id()).or_else(|| {
        state()
            .content_view
            .lock()
            .ok()
            .and_then(|g| g.as_ref().map(|h| h.0.clone()))
    })
}

/// The shell view handle (UI thread only).
pub fn shell_view() -> Option<cef::BrowserView> {
    state()
        .shell_view
        .lock()
        .ok()
        .and_then(|g| g.as_ref().map(|h| h.0.clone()))
}

/// Sends the current native tab snapshot to the trusted chrome document.
/// This is deliberately pushed from the CEF UI thread instead of polled from
/// JavaScript, so address-bar updates do not create a steady stream of bridge
/// requests or race the BrowserView state across threads.
pub fn notify_shell_state() {
    refresh_window_title();
    remember_session();
    let payload = serde_json::json!({
        "activeTabId": active_tab_id(),
        "tabs": tabs_snapshot(),
    });
    let Ok(payload) = serde_json::to_string(&payload) else {
        return;
    };
    let script = format!("window.__agentgoNativeTabs && window.__agentgoNativeTabs({payload});");
    let Some(view) = shell_view() else {
        return;
    };
    if let Some(browser) = view.browser() {
        if let Some(frame) = browser.main_frame() {
            frame.execute_java_script(
                Some(&cef::CefString::from(script.as_str())),
                Some(&cef::CefString::from("agentgo://shell/chrome.html")),
                0,
            );
        }
    }
}

/// Stores the trusted browser-level utility BrowserView.
pub fn set_tool_view(view: cef::BrowserView) {
    if let Ok(mut slot) = state().tool_view.lock() {
        *slot = Some(UiHandle(view));
    }
}

fn tool_view() -> Option<cef::BrowserView> {
    state()
        .tool_view
        .lock()
        .ok()
        .and_then(|slot| slot.as_ref().map(|handle| handle.0.clone()))
}

/// Stores the native controller for the tightly bounded utility surface.
pub fn set_tool_overlay(overlay: cef::OverlayController) {
    if let Ok(mut slot) = state().tool_overlay.lock() {
        *slot = Some(UiHandle(overlay));
    }
}

/// Recreates the bounded utility overlay after a content BrowserView is added.
/// CEF guarantees an overlay is above child Views that already exist when the
/// overlay is created; tabs created later can otherwise cover browser tools.
/// Keeping this repair at the native composition boundary makes History,
/// Downloads, and Settings available over every tab without exposing a bridge
/// to page content.
pub fn restack_tool_overlay() -> Result<(), String> {
    let window = state()
        .window
        .lock()
        .ok()
        .and_then(|slot| slot.as_ref().map(|handle| handle.0.clone()))
        .ok_or_else(|| "composition window is not ready".to_string())?;
    let tools = tool_view().ok_or_else(|| "browser tool surface is not ready".to_string())?;
    let was_open = current_tool().is_some();
    // The agent lock layer goes back above the tabs first, the panels above it.
    if let Some(layer) = LOCK_LAYER.with(|layer| layer.borrow_mut().take()) {
        layer.set_visible(0);
        layer.destroy();
        make_lock_layer();
        position_lock_layer();
        LOCK_LAYER.with(|layer| {
            if let Some(overlay) = layer.borrow().as_ref() {
                overlay.set_visible(i32::from(agent_locked()));
            }
        });
    }

    if let Some(overlay) = tool_overlay() {
        overlay.set_visible(0);
        overlay.destroy();
    }
    tools.set_visible(0);

    let mut as_view = View::from(&tools);
    let overlay = window
        .add_overlay_view(
            Some(&mut as_view),
            DockingMode::CUSTOM,
            i32::from(TOOL_OVERLAY_ACTIVATABLE.with(std::cell::Cell::get)),
        )
        .ok_or_else(|| "CEF could not restack the browser tool surface".to_string())?;
    overlay.set_visible(0);
    set_tool_overlay(overlay.clone());

    if was_open {
        relayout_tool_surface();
        tools.set_enabled(1);
        tools.set_visible(1);
        overlay.set_visible(1);
    }
    log_event("AG-RUNTIME tools-overlay-restacked");
    Ok(())
}

fn tool_overlay() -> Option<cef::OverlayController> {
    state()
        .tool_overlay
        .lock()
        .ok()
        .and_then(|slot| slot.as_ref().map(|handle| handle.0.clone()))
}

fn current_tool() -> Option<String> {
    state()
        .active_tool
        .lock()
        .ok()
        .and_then(|tool| tool.clone())
}

fn notify_shell_tool_state(tool: Option<&str>) {
    let payload = serde_json::to_string(&tool).unwrap_or_else(|_| "null".to_string());
    let script = format!("window.__agentgoToolState && window.__agentgoToolState({payload});");
    let Some(view) = shell_view() else {
        return;
    };
    if let Some(browser) = view.browser() {
        if let Some(frame) = browser.main_frame() {
            frame.execute_java_script(
                Some(&cef::CefString::from(script.as_str())),
                Some(&cef::CefString::from("agentgo://shell/chrome.html")),
                0,
            );
        }
    }
}

fn execute_tool_script(script: &str) -> bool {
    let Some(view) = tool_view() else {
        return false;
    };
    let Some(browser) = view.browser() else {
        return false;
    };
    let Some(frame) = browser.main_frame() else {
        return false;
    };
    frame.execute_java_script(
        Some(&cef::CefString::from(script)),
        Some(&cef::CefString::from("agentgo://shell/tools.html")),
        0,
    );
    true
}

thread_local! {
    /// The rectangle the panels were last placed at, in window coordinates.
    static TOOL_RECT: std::cell::Cell<(i32, i32)> = const { std::cell::Cell::new((0, 0)) };
    /// Width and height of the same rectangle.
    static TOOL_SIZE: std::cell::Cell<(i32, i32)> = const { std::cell::Cell::new((0, 0)) };
}

fn place_tools(overlay: &cef::OverlayController, rect: &cef::Rect) {
    TOOL_RECT.with(|cell| cell.set((rect.x, rect.y)));
    TOOL_SIZE.with(|cell| cell.set((rect.width, rect.height)));
    overlay.set_bounds(Some(rect));
}

/// Repositions the active utility after a window resize. Only the opaque
/// panel rectangle is overlaid, so the surrounding page remains visible.
pub fn relayout_tool_surface() {
    position_lock_layer();
    reposition_devtools();
    let Some(tool) = current_tool() else {
        return;
    };
    let Some(window) = state()
        .window
        .lock()
        .ok()
        .and_then(|slot| slot.as_ref().map(|handle| handle.0.clone()))
    else {
        return;
    };
    let Some(overlay) = tool_overlay() else {
        return;
    };
    let bounds = window.bounds();
    let chrome = chrome_height();
    let available_height = (bounds.height - chrome).max(1);
    let panel = if tool == "settings" {
        let width = 720.min(bounds.width.max(1));
        cef::Rect {
            x: bounds.width.saturating_sub(width),
            y: chrome.min(bounds.height.saturating_sub(1)),
            width,
            height: available_height,
        }
    } else {
        let horizontal_margin = 16.min((bounds.width / 4).max(0));
        let vertical_margin = 10.min((available_height / 4).max(0));
        if tool == "siteinfo" {
            // Under the site chip at the left of the address bar.
            let (x, _, _) = SUGGEST.with(|s| *s.borrow());
            place_tools(
                &overlay,
                &cef::Rect {
                    x: x.clamp(0, bounds.width.saturating_sub(400).max(0)),
                    y: 100,
                    width: 400.min(bounds.width.max(1)),
                    height: 360.min(available_height),
                },
            );
            return;
        }
        if tool == "palette" || tool == "setup" {
            let (want_width, want_height) = if tool == "palette" {
                (660, 470)
            } else {
                (740, 600)
            };
            let width = want_width.min((bounds.width - 24).max(1));
            let height = want_height.min((available_height - 24).max(1));
            let top = if tool == "palette" {
                chrome + 18
            } else {
                chrome + ((available_height - height) / 2).max(8)
            };
            place_tools(
                &overlay,
                &cef::Rect {
                    x: ((bounds.width - width) / 2).max(0),
                    y: top,
                    width,
                    height,
                },
            );
            return;
        }
        if tool == "translate" {
            let width = 380.min(bounds.width.max(1));
            place_tools(
                &overlay,
                &cef::Rect {
                    x: (bounds.width - width - 16).max(0),
                    y: 100,
                    width,
                    height: 330.min(available_height),
                },
            );
            return;
        }
        if tool == "permission" {
            let width = 400.min(bounds.width.max(1));
            place_tools(
                &overlay,
                &cef::Rect {
                    x: 16.min((bounds.width - width).max(0)),
                    y: 100,
                    width,
                    height: 250.min(available_height),
                },
            );
            return;
        }
        if tool == "suggest" {
            // Right below the address bar, as wide as it.
            let (x, width, rows) = SUGGEST.with(|s| *s.borrow());
            place_tools(
                &overlay,
                &cef::Rect {
                    x: x.clamp(0, bounds.width.saturating_sub(200).max(0)),
                    y: 100,
                    width: width.clamp(200, bounds.width.max(200)),
                    height: (rows * 46 + 12).clamp(58, 440).min(available_height),
                },
            );
            return;
        }
        if tool == "ctxmenu" {
            let (x, y, w, h) = CTX_RECT.with(std::cell::Cell::get);
            if x < 0 {
                let width = 420.min(bounds.width.max(1));
                place_tools(
                    &overlay,
                    &cef::Rect {
                        x: (bounds.width - width - 12).max(0),
                        y: chrome + 8,
                        width,
                        height: (available_height - 16).clamp(1, 560),
                    },
                );
            } else {
                // The docked developer tools cover the page's right side.
                let right = bounds.width - devtools_reserved(bounds.width);
                let width = w.min(right.max(1));
                let height = h.min(available_height.max(1));
                place_tools(
                    &overlay,
                    &cef::Rect {
                        x: x.clamp(0, (right - width).max(0)),
                        y: y.clamp(chrome, (bounds.height - height).max(chrome)),
                        width,
                        height,
                    },
                );
            }
            return;
        }
        let (want_width, want_height) = match tool.as_str() {
            "agent" => (480, 640),
            "find" => (420, 56),
            "menu" => (300, 610),
            "extensions" => (400, 560),
            "report" => (500, 660),
            _ => (440, 520),
        };
        let width = want_width.min((bounds.width - horizontal_margin * 2).max(1));
        let height = want_height.min((available_height - vertical_margin * 2).max(1));
        cef::Rect {
            x: (bounds.width - width - horizontal_margin).max(0),
            y: chrome + vertical_margin,
            width,
            height,
        }
    };
    place_tools(&overlay, &panel);
}

fn present_tool_on_ui(tool: &str) {
    TOOL_GENERATION.with(|g| g.set(g.get() + 1));
    // Address suggestions sit in an overlay that cannot be activated, so the
    // person keeps typing in the address bar; the other panels take the
    // keyboard (find, settings search). Switching rebuilds the overlay.
    let activatable = tool != "suggest";
    if TOOL_OVERLAY_ACTIVATABLE.with(std::cell::Cell::get) != activatable {
        TOOL_OVERLAY_ACTIVATABLE.with(|cell| cell.set(activatable));
        if let Err(error) = restack_tool_overlay() {
            log_event(&format!("AG-RUNTIME tools-overlay-mode-failed {error}"));
        }
    }
    relayout_tool_surface();
    if let Some(view) = tool_view() {
        view.set_enabled(1);
        view.set_visible(1);
        if tool != "suggest" {
            // Typed keys go to the panel, as in any browser's find bar; and
            // the page, no longer holding the focus, reports the person's next
            // click (content_clicked) so the panel can slide away.
            view.request_focus();
            if let Some(host) = view.browser().and_then(|browser| browser.host()) {
                host.set_focus(1);
            }
        }
    }
    if let Some(overlay) = tool_overlay() {
        overlay.set_visible(1);
    }
    let payload = serde_json::to_string(tool).unwrap_or_else(|_| "null".to_string());
    execute_tool_script(&format!(
        "window.__agentgoOpenTool && window.__agentgoOpenTool({payload});"
    ));
    notify_shell_tool_state(Some(tool));
    log_event(&format!("AG-RUNTIME tool-opened {tool}"));
}

thread_local! {
    /// Whether the tools overlay may take activation (see present_tool_on_ui).
    static TOOL_OVERLAY_ACTIVATABLE: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
}

fn open_tool_on_ui(tool: &str) {
    if current_tool().as_deref() == Some(tool) {
        if !execute_tool_script("window.__agentgoRequestClose && window.__agentgoRequestClose();") {
            close_tool_on_ui();
        }
        return;
    }
    if let Ok(mut active) = state().active_tool.lock() {
        *active = Some(tool.to_string());
    }
    present_tool_on_ui(tool);
}

fn close_tool_on_ui() {
    if current_tool().as_deref() == Some("find") {
        if let Some(host) = content_browser_host_for_tab(&active_tab_id()) {
            host.stop_finding(1);
        }
    }
    if let Some(overlay) = tool_overlay() {
        overlay.set_visible(0);
    }
    if let Some(view) = tool_view() {
        view.set_visible(0);
    }
    if let Ok(mut active) = state().active_tool.lock() {
        *active = None;
    }
    notify_shell_tool_state(None);
    log_event("AG-RUNTIME tool-closed");
}

/// Replays an already-requested utility once tools.html finishes loading.
pub fn refresh_tool_surface() {
    let Some(tool) = current_tool() else {
        if let Some(overlay) = tool_overlay() {
            overlay.set_visible(0);
        }
        return;
    };
    present_tool_on_ui(&tool);
}

/// Opens or toggles a browser utility panel (UI thread).
pub fn open_tool(tool: &str) -> Result<serde_json::Value, String> {
    if !matches!(
        tool,
        "history"
            | "downloads"
            | "settings"
            | "agent"
            | "accounts"
            | "savelogin"
            | "find"
            | "menu"
            | "bookmarks"
            | "extensions"
            | "suggest"
            | "siteinfo"
            | "ctxmenu"
            | "palette"
            | "permission"
            | "translate"
            | "setup"
            | "report"
    ) {
        return Err(format!("unknown browser tool {tool}"));
    }
    open_tool_on_ui(tool);
    Ok(serde_json::json!({"accepted":true,"tool":tool}))
}

thread_local! {
    /// Address bar left edge, width and suggestion count, for the popover.
    static SUGGEST: std::cell::RefCell<(i32, i32, i32)> = const { std::cell::RefCell::new((240, 640, 6)) };
}

/// The address bar's suggestion popover (`ui.suggest`): shows `items` under
/// the bar, or closes it when there are none.
pub fn show_suggestions(x: i32, width: i32, items: &serde_json::Value, selected: i64, text: &str) {
    let rows = items.as_array().map_or(0, Vec::len) as i32;
    if rows == 0 {
        if current_tool().as_deref() == Some("suggest") {
            close_tool_on_ui();
        }
        return;
    }
    SUGGEST.with(|s| *s.borrow_mut() = (x, width, rows));
    let _ = show_tool(
        "suggest",
        "",
        &serde_json::json!({"items": items, "selected": selected, "text": text}),
    );
    relayout_tool_surface();
}

/// Shows a panel (never toggles it closed) and scrolls to `section` in it.
/// `data` is handed to the panel as `window.__agentgoShellData`.
pub fn show_tool(tool: &str, section: &str, data: &serde_json::Value) -> Result<(), String> {
    let payload = serde_json::json!({"section": section, "data": data}).to_string();
    let script = format!("window.__agentgoShellData = {payload};");
    if current_tool().as_deref() == Some(tool) {
        execute_tool_script(&format!(
            "{script} window.__agentgoRefresh && window.__agentgoRefresh();"
        ));
        return Ok(());
    }
    execute_tool_script(&script);
    open_tool(tool).map(|_| ())
}

/// Runs a script in the trusted chrome document (address bar, tab strip).
pub fn chrome_script(script: &str) -> bool {
    let Some(frame) = shell_view()
        .and_then(|view| view.browser())
        .and_then(|browser| browser.main_frame())
    else {
        return false;
    };
    frame.execute_java_script(
        Some(&cef::CefString::from(script)),
        Some(&cef::CefString::from("agentgo://shell/chrome.html")),
        0,
    );
    true
}

/// Height of the two toolbar decks, and of the favorites bar under them.
const CHROME_BASE: i32 = 104;
const FAVORITES_BAR: i32 = 36;

/// Height of the trusted chrome: the favorites bar adds a row when it is on.
pub fn chrome_height() -> i32 {
    let bar = settings_snapshot()["favoritesBar"] != false;
    CHROME_BASE + if bar { FAVORITES_BAR } else { 0 }
}

/// Re-lays the window out after the favorites bar was turned on or off.
fn apply_chrome_height() {
    super::exec::post(|| {
        let (Some(shell), Some(window)) = (
            shell_view(),
            state()
                .window
                .lock()
                .ok()
                .and_then(|slot| slot.as_ref().map(|handle| handle.0.clone())),
        ) else {
            return;
        };
        cef::View::from(&shell).invalidate_layout();
        window.layout();
        relayout_tool_surface();
    });
    wake_ui();
}

/// Panels that go away when the person clicks the page behind them. Find
/// stays (people search while they read), and suggestions follow the address
/// bar's own focus.
fn dismissed_by_page_click(tool: &str) -> bool {
    !matches!(tool, "find" | "suggest" | "setup" | "permission")
}

/// Command palette and setup wizard actions (UI thread). A name of the form
/// `tool:<panel>` opens a panel (on a settings section when `section` is
/// given); anything else is a browser shortcut or the Inspect command.
pub fn run_action(name: &str, section: &str) -> Result<serde_json::Value, String> {
    if let Some(tool) = name.strip_prefix("tool:") {
        close_tool_on_ui();
        show_tool(tool, section, &serde_json::json!({}))?;
    } else if name == "inspect" {
        close_tool_on_ui();
        context_action(&serde_json::json!({"action": "inspect"}))?;
    } else if name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    {
        close_tool_on_ui();
        shell_shortcut(name);
    } else {
        return Err(format!("unknown action {name}"));
    }
    Ok(serde_json::json!({"accepted": true}))
}

/// Whether a request box may appear now (nothing else is open).
pub fn tool_is_free_for_prompt() -> bool {
    matches!(current_tool().as_deref(), None | Some("permission"))
}

/// Smoke runs never show boxes that wait for a person.
pub fn is_smoke_run() -> bool {
    state().options.smoke
}

/// Shows the setup wizard once, the first time a normal profile starts.
pub fn maybe_first_run() {
    let skip = std::env::var_os("AGENTGO_SKIP_SETUP").is_some()
        || state().options.smoke
        || state().ephemeral_profile.load(Ordering::SeqCst)
        || settings_snapshot()["setupDone"] == true;
    if skip {
        return;
    }
    super::exec::spawn(async {
        super::exec::sleep(1_200).await;
        if current_tool().is_none() {
            open_tool_on_ui("setup");
        }
    });
}

/// The page took the keyboard because the person clicked it: the open panel
/// slides away with its own closing animation. Focus the page gets from a
/// script or an agent does not close anything.
pub fn content_clicked() {
    let Some(tool) = current_tool() else { return };
    if !dismissed_by_page_click(&tool) {
        return;
    }
    if !execute_tool_script("window.__agentgoRequestClose && window.__agentgoRequestClose();") {
        close_tool_on_ui();
    }
}

/// A left click landed on `hwnd` (from the UI thread's mouse hook). When it
/// is the active page's own window, that is a click on the website.
#[cfg(target_os = "windows")]
pub fn page_mouse_down(hwnd: isize, point: (i32, i32)) {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetWindowRect, IsChild};
    // The hook is live before the state exists; a panic inside it would abort.
    if STATE.get().is_none() || current_tool().is_none() {
        return;
    }
    let Some(host) = content_host(&active_tab_id()) else {
        return;
    };
    let page = host.window_handle().0 as isize;
    if page == 0 {
        return;
    }
    // SAFETY: plain handle comparison; both are windows of this thread.
    let inside = hwnd == page || unsafe { IsChild(page as _, hwnd as _) } != 0;
    // A click on the open panel itself is not a click on the website. Every
    // view shares the one native window, so the panel's own rectangle (set when
    // it was placed, in DIPs) is mapped onto the window's pixels.
    let on_panel = {
        let (x, y) = TOOL_RECT.with(std::cell::Cell::get);
        let (w, h) = TOOL_SIZE.with(std::cell::Cell::get);
        let width_dip = state()
            .window
            .lock()
            .ok()
            .and_then(|slot| slot.as_ref().map(|handle| handle.0.bounds().width))
            .unwrap_or(0);
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        // SAFETY: the rectangle is a valid out-pointer for the call.
        let got = unsafe { GetWindowRect(page as _, &mut rect) } != 0;
        if !got || width_dip <= 0 || w <= 0 {
            false
        } else {
            let scale = f64::from(rect.right - rect.left) / f64::from(width_dip);
            let px = |value: i32| (f64::from(value) * scale).round() as i32;
            let (left, top) = (rect.left + px(x), rect.top + px(y));
            point.0 >= left && point.0 < left + px(w) && point.1 >= top && point.1 < top + px(h)
        }
    };
    if inside && !on_panel {
        content_clicked();
    }
}

/// Hides the utility surface after tools.js completes its close transition.
pub fn close_tool() -> serde_json::Value {
    // The panel's contents have already left; now the slab itself goes:
    // it rolls up into the toolbar (the settings drawer slides out right).
    // Shrinking never exposes unpainted area, so nothing flashes.
    let (Some(overlay), Some(tool)) = (tool_overlay(), current_tool()) else {
        close_tool_on_ui();
        return serde_json::json!({"accepted":true,"closing":true});
    };
    let generation = TOOL_GENERATION.with(std::cell::Cell::get);
    let start = overlay.bounds();
    super::exec::spawn(async move {
        // Time-based, so a slow frame skips ahead instead of stretching
        // the whole motion.
        const DURATION_MS: f64 = 170.0;
        let began = std::time::Instant::now();
        loop {
            super::exec::sleep(1).await;
            if TOOL_GENERATION.with(std::cell::Cell::get) != generation {
                return; // another panel opened meanwhile
            }
            let elapsed = began.elapsed().as_secs_f64() * 1000.0;
            let done = elapsed >= DURATION_MS;
            // ease-in: slow start, quick finish, like a blind snapping up
            let t = (elapsed / DURATION_MS).min(1.0);
            let left = 1.0 - t * t * t;
            let bounds = if tool == "settings" {
                let width = (f64::from(start.width) * left).round() as i32;
                cef::Rect {
                    x: start.x + start.width - width,
                    y: start.y,
                    width: width.max(1),
                    height: start.height,
                }
            } else {
                cef::Rect {
                    x: start.x,
                    y: start.y,
                    width: start.width,
                    height: ((f64::from(start.height) * left).round() as i32).max(1),
                }
            };
            overlay.set_bounds(Some(&bounds));
            if done {
                break;
            }
        }
        if TOOL_GENERATION.with(std::cell::Cell::get) == generation {
            close_tool_on_ui();
        }
    });
    serde_json::json!({"accepted":true,"closing":true})
}

thread_local! {
    /// Counts panel openings, so a closing animation stops if one opens.
    static TOOL_GENERATION: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Shows the agent panel when an approval or handoff needs the operator.
pub fn present_agent_panel() {
    if current_tool().as_deref() != Some("agent") {
        open_tool_on_ui("agent");
    } else {
        execute_tool_script("window.__agentgoRefresh && window.__agentgoRefresh();");
    }
}

/// Pushes the live agent/session state to the trusted chrome and tools.
pub fn notify_shell_agent() {
    super::fingerprint::refresh_all();
    let Ok(payload) = serde_json::to_string(&super::sessions::status()) else {
        return;
    };
    if let Some(frame) = shell_view()
        .and_then(|view| view.browser())
        .and_then(|browser| browser.main_frame())
    {
        frame.execute_java_script(
            Some(&cef::CefString::from(
                format!("window.__agentgoAgent && window.__agentgoAgent({payload});").as_str(),
            )),
            Some(&cef::CefString::from("agentgo://shell/chrome.html")),
            0,
        );
    }
    if current_tool().as_deref() == Some("agent") {
        execute_tool_script("window.__agentgoRefresh && window.__agentgoRefresh();");
    }
}

/// Reads the zoom of a tab, after setting it when `percent` is given. Chromium
/// counts zoom in steps of 20 %: level = log1.2(percent / 100).
pub fn zoom_tab(tab_id: &str, percent: Option<f64>) -> Result<f64, String> {
    let host = content_browser_host_for_tab(tab_id)
        .ok_or_else(|| "content browser is not ready".to_string())?;
    if let Some(percent) = percent {
        host.set_zoom_level((percent / 100.0).ln() / 1.2_f64.ln());
        notify_shell_state();
        return Ok(percent.round());
    }
    Ok((1.2_f64.powf(host.zoom_level()) * 100.0).round())
}

/// The find bar: Chromium's own find in the visible tab. Results arrive in
/// [`find_result`].
pub fn find_in_page(text: &str, forward: bool, match_case: bool, next: bool) -> Result<(), String> {
    let host = content_browser_host_for_tab(&active_tab_id())
        .ok_or_else(|| "content browser is not ready".to_string())?;
    if text.is_empty() {
        host.stop_finding(1);
    } else {
        host.find(
            Some(&CefString::from(text)),
            forward as i32,
            match_case as i32,
            next as i32,
        );
    }
    Ok(())
}

/// Match count and position for the find bar.
pub fn find_result(count: i32, ordinal: i32) {
    if current_tool().as_deref() == Some("find") {
        execute_tool_script(&format!(
            "window.__agentgoFindResult && window.__agentgoFindResult({count}, {ordinal});"
        ));
    }
}

cef::wrap_pdf_print_callback! {
    struct PrintReady {
        path: String,
    }

    impl PdfPrintCallback {
        fn on_pdf_print_finished(&self, _path: Option<&CefString>, ok: ::std::os::raw::c_int) {
            if ok == 0 {
                log_event("AG-RUNTIME print-failed");
                return;
            }
            // The page is saved as a PDF and opened in the PDF viewer
            // Windows has, which has a working Print button. The embedded
            // engine cannot show the system print dialog itself.
            if let Err(error) = super::extras::shell_open(&self.path, None) {
                log_event(&format!("AG-RUNTIME print-open-failed {}", error.message));
            }
        }
    }
}

/// Print: the active page becomes a PDF that opens in the system viewer.
pub fn print_active_tab() {
    let Some(host) = content_browser_host_for_tab(&active_tab_id()) else {
        return;
    };
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let path = std::env::temp_dir()
        .join(format!("AgentGo-print-{stamp}.pdf"))
        .display()
        .to_string();
    let settings = cef::PdfPrintSettings {
        scale: 1.0,
        paper_width: 8.27,
        paper_height: 11.69,
        print_background: 1,
        ..Default::default()
    };
    host.print_to_pdf(
        Some(&CefString::from(path.as_str())),
        Some(&settings),
        Some(&mut PrintReady::new(path.clone())),
    );
}

thread_local! {
    /// Where the right-click panel goes: x, y, width, height in window
    /// coordinates. A negative x means the report layout (a tall panel at the
    /// right edge).
    static CTX_RECT: std::cell::Cell<(i32, i32, i32, i32)> = const { std::cell::Cell::new((0, 0, 240, 300)) };
}

/// What the page's right-click offers, decided here so the panel's size is
/// known before it is drawn.
fn context_items(data: &serde_json::Value) -> Vec<serde_json::Value> {
    let item = |action: &str, label: &str, disabled: bool| serde_json::json!({"action": action, "label": label, "disabled": disabled});
    let sep = || serde_json::json!({"sep": true});
    let text = |key: &str| data[key].as_str().unwrap_or("").to_string();
    let tab = text("tabId");
    let (can_back, can_forward) = content_view_for_tab(&tab)
        .and_then(|view| view.browser())
        .map_or((false, false), |browser| {
            (browser.can_go_back() != 0, browser.can_go_forward() != 0)
        });
    let mut items = Vec::new();
    if data["editable"] == true {
        items.extend([
            item("cut", "Cut", false),
            item("copy", "Copy", false),
            item("paste", "Paste", false),
            item("select_all", "Select all", false),
            sep(),
        ]);
    } else if !text("selection").is_empty() {
        items.extend([
            item("copy_text", "Copy", false),
            item("translate", "Translate selection", false),
            sep(),
        ]);
    }
    if !text("link").is_empty() {
        items.extend([
            item("open_link", "Open link in new tab", false),
            item("copy_link", "Copy link address", false),
            sep(),
        ]);
    }
    if data["image"] == true && !text("source").is_empty() {
        items.extend([
            item("open_image", "Open image in new tab", false),
            item("copy_image", "Copy image address", false),
            sep(),
        ]);
    }
    items.extend([
        item("back", "Back", !can_back),
        item("forward", "Forward", !can_forward),
        item("reload", "Reload", false),
        sep(),
        item("translate_page", "Translate page", false),
        item("print", "Print…", false),
        item("inspect", "Inspect", false),
        sep(),
        item("a11y", "Check accessibility", false),
        item("fonts", "Identify font", false),
    ]);
    items
}

/// The right-click menu, drawn in the browser's own theme in the tools
/// overlay at the pointer (UI thread).
pub fn show_context_menu(mut data: serde_json::Value) {
    let items = context_items(&data);
    let rows = items.iter().filter(|i| i["sep"] != true).count() as i32;
    let seps = items.len() as i32 - rows;
    let x = data["x"].as_i64().unwrap_or(0) as i32;
    let y = data["y"].as_i64().unwrap_or(0) as i32;
    CTX_RECT.with(|r| r.set((x, y + chrome_height(), 272, 18 + rows * 36 + seps * 12)));
    data["items"] = serde_json::Value::Array(items);
    let payload = serde_json::json!({"mode": "menu", "menu": data});
    if current_tool().as_deref() == Some("ctxmenu") {
        // A second right-click while the menu is open or still sliding away:
        // cancel that close and open the menu again where the pointer is.
        TOOL_GENERATION.with(|g| g.set(g.get() + 1));
        let data = serde_json::json!({"section": "", "data": payload}).to_string();
        relayout_tool_surface();
        execute_tool_script(&format!(
            "window.__agentgoShellData = {data}; window.__agentgoOpenTool && window.__agentgoOpenTool('ctxmenu');"
        ));
        return;
    }
    let _ = show_tool("ctxmenu", "", &payload);
    relayout_tool_surface();
}

/// Replaces the menu with a result panel (accessibility report, font info).
fn show_context_report(report: serde_json::Value) {
    CTX_RECT.with(|r| {
        let (x, y, _, _) = r.get();
        // A translation is a card where the menu was; the audits need a tall panel.
        if report["kind"] == "translate" && x >= 0 {
            r.set((x, y, 380, 300));
        } else {
            r.set((-1, 0, 420, 0));
        }
    });
    let _ = show_tool(
        "ctxmenu",
        "",
        &serde_json::json!({"mode": "report", "report": report}),
    );
    relayout_tool_surface();
}

/// Puts text on the Windows clipboard (the person's own, by their click).
fn clipboard_set_text(text: &str) {
    use windows_sys::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
    };
    use windows_sys::Win32::System::Memory::{
        GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE,
    };
    let wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    // SAFETY: the buffer is sized for `wide`, locked while copied, and handed to
    // the clipboard, which owns it once SetClipboardData succeeds.
    unsafe {
        if OpenClipboard(std::ptr::null_mut()) == 0 {
            return;
        }
        EmptyClipboard();
        let handle = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2);
        if !handle.is_null() {
            let ptr = GlobalLock(handle) as *mut u16;
            if !ptr.is_null() {
                std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr, wide.len());
                GlobalUnlock(handle);
                SetClipboardData(13, handle);
            }
        }
        CloseClipboard();
    }
}

/// Developer tools per tab: their native window and whether they are shown.
/// Closing only hides them. In this engine, tearing a developer tools window
/// down also tore down the toolbar's surface, so they live as long as their
/// tab and are shown again when asked.
static DEVTOOLS: std::sync::Mutex<Vec<(String, isize, bool)>> = std::sync::Mutex::new(Vec::new());
/// The tab whose developer tools are being opened (until they load), and
/// the point to inspect.
static DEVTOOLS_PENDING: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
static DEVTOOLS_POINT: std::sync::Mutex<(i32, i32)> = std::sync::Mutex::new((0, 0));
/// A hidden developer tools window that stays open from the first Inspect
/// on: 0 none, 1 opening, 2 open. Developer tools windows are the engine's
/// only Chrome-style windows, and when the last of them closed the engine
/// began shutting the other views down (the toolbar went blank). With this
/// one always there, closing a tab with its tools open is safe.
static DEVTOOLS_ANCHOR: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// Whether developer tools being created now are the hidden anchor.
pub fn devtools_anchor_opening() -> bool {
    DEVTOOLS_ANCHOR.load(std::sync::atomic::Ordering::Relaxed) == 1
}

/// Hides a developer tools window (the anchor) as soon as it exists.
#[cfg(target_os = "windows")]
pub fn hide_devtools_window(own: isize) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetAncestor, ShowWindow, GA_ROOT, SW_HIDE};
    // SAFETY: a window of this process; only its visibility changes.
    unsafe {
        let root = GetAncestor(own as _, GA_ROOT);
        ShowWindow(if root.is_null() { own as _ } else { root }, SW_HIDE);
    }
}

/// Opens the developer tools of `tab`, after the anchor when there is none.
fn open_devtools(tab: &str, x: i32, y: i32) {
    if let Ok(mut pending) = DEVTOOLS_PENDING.lock() {
        *pending = Some(tab.to_string());
    }
    if let Ok(mut point) = DEVTOOLS_POINT.lock() {
        *point = (x, y);
    }
    match DEVTOOLS_ANCHOR.load(std::sync::atomic::Ordering::Relaxed) {
        2 => {
            if let Some(host) = content_host(tab) {
                dock_devtools(&host, x, y);
            }
        }
        1 => {}
        _ => {
            let anchor = tool_view()
                .and_then(|view| view.browser())
                .and_then(|b| b.host());
            match anchor {
                Some(host) => {
                    DEVTOOLS_ANCHOR.store(1, std::sync::atomic::Ordering::Relaxed);
                    dock_devtools(&host, 0, 0);
                }
                None => {
                    if let Some(host) = content_host(tab) {
                        dock_devtools(&host, x, y);
                    }
                }
            }
        }
    }
}

#[cfg(target_os = "windows")]
fn window_alive(hwnd: isize) -> bool {
    // SAFETY: a plain handle validity check.
    hwnd != 0 && unsafe { windows_sys::Win32::UI::WindowsAndMessaging::IsWindow(hwnd as _) } != 0
}

/// The docked tools of a tab, when they exist: (window, shown).
fn devtools_of(tab: &str) -> Option<(isize, bool)> {
    let mut all = DEVTOOLS.lock().ok()?;
    all.retain(|(_, hwnd, _)| window_alive(*hwnd));
    all.iter()
        .find(|(t, _, _)| t == tab)
        .map(|(_, hwnd, shown)| (*hwnd, *shown))
}

fn set_devtools_shown(tab: &str, shown: bool) {
    if let Ok(mut all) = DEVTOOLS.lock() {
        for entry in all.iter_mut().filter(|(t, _, _)| t == tab) {
            entry.2 = shown;
        }
    }
}
/// Share of the window width the docked developer tools take.
const DEVTOOLS_SHARE: f64 = 0.36;

#[cfg(target_os = "windows")]
#[link(name = "user32")]
extern "system" {
    fn GetDpiForWindow(window: *mut std::ffi::c_void) -> u32;
}

/// The pixel rectangle (x, y, width, height) of the dock inside `window`.
#[cfg(target_os = "windows")]
fn devtools_dock_rect(window: isize) -> Option<(i32, i32, i32, i32)> {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowRect;
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    // SAFETY: the rectangle is a valid out-pointer for the call.
    if unsafe { GetWindowRect(window as _, &mut rect) } == 0 {
        return None;
    }
    let (width, height) = (rect.right - rect.left, rect.bottom - rect.top);
    let dock = (f64::from(width) * DEVTOOLS_SHARE) as i32;
    // SAFETY: a plain query of the window's DPI.
    let dpi = unsafe { GetDpiForWindow(window as _) }.max(96) as i32;
    let top = (chrome_height() * dpi / 96).min(height);
    Some((width - dock, top, dock, height - top))
}

/// Opens the developer tools docked at the bottom of the same window.
#[cfg(target_os = "windows")]
fn dock_devtools(host: &cef::BrowserHost, x: i32, y: i32) {
    // The engine opens its own window; `devtools_loaded` moves it into this one.
    let info = cef::WindowInfo::default();
    host.show_dev_tools(
        Some(&info),
        Some(&mut super::app::devtools_client()),
        None,
        Some(&cef::Point { x, y }),
    );
}

/// Closes (hides) the developer tools of the active tab.
pub fn close_devtools() {
    set_devtools_shown(&active_tab_id(), false);
    super::app::relayout_registered_content();
    log_event("AG-RUNTIME devtools-hidden");
}

/// Height (in the units of `total`) the docked developer tools cover, 0 when closed.
fn devtools_reserved(total: i32) -> i32 {
    if devtools_of(&active_tab_id()).is_some_and(|(_, shown)| shown) {
        (f64::from(total) * DEVTOOLS_SHARE) as i32
    } else {
        0
    }
}

/// Turns the developer tools' own top-level window into a child of the browser
/// window, pinned to the bottom. Returns the window that was moved.
#[cfg(target_os = "windows")]
fn adopt_devtools_window(own: isize) -> isize {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetAncestor, GetWindowLongPtrW, SetParent, SetWindowLongPtrW, SetWindowPos, GA_ROOT,
        GWL_STYLE, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_SHOWWINDOW, WS_CAPTION, WS_CHILD,
        WS_CLIPSIBLINGS, WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_POPUP, WS_SYSMENU, WS_THICKFRAME,
    };
    let Some(host) = content_host(&active_tab_id()) else {
        return own;
    };
    let parent = host.window_handle().0 as isize;
    // SAFETY: both handles belong to this process; the calls only change window styles and placement.
    unsafe {
        let root = GetAncestor(own as _, GA_ROOT) as isize;
        if root == 0 || root == parent {
            return own;
        }
        let style = GetWindowLongPtrW(root as _, GWL_STYLE) as u32;
        let cleared = style
            & !(WS_POPUP
                | WS_CAPTION
                | WS_THICKFRAME
                | WS_SYSMENU
                | WS_MINIMIZEBOX
                | WS_MAXIMIZEBOX);
        SetWindowLongPtrW(
            root as _,
            GWL_STYLE,
            (cleared | WS_CHILD | WS_CLIPSIBLINGS) as _,
        );
        SetParent(root as _, parent as _);
        if let Some((x, y, w, h)) = devtools_dock_rect(parent) {
            SetWindowPos(
                root as _,
                std::ptr::null_mut(),
                x,
                y,
                w,
                h,
                SWP_FRAMECHANGED | SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
        }
        root
    }
}

/// Shows the active tab's developer tools (when they are open) beside the
/// page, and hides every other tab's.
#[cfg(target_os = "windows")]
fn reposition_devtools() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SetWindowPos, ShowWindow, SWP_NOACTIVATE, SWP_NOZORDER, SWP_SHOWWINDOW, SW_HIDE,
    };
    let active = active_tab_id();
    let entries: Vec<(String, isize, bool)> = DEVTOOLS
        .lock()
        .map(|mut all| {
            all.retain(|(_, hwnd, _)| window_alive(*hwnd));
            all.clone()
        })
        .unwrap_or_default();
    let parent = content_host(&active).map(|host| host.window_handle().0 as isize);
    for (tab, hwnd, shown) in entries {
        // SAFETY: windows of this process; only their placement and visibility change.
        unsafe {
            match (tab == active && shown, parent.and_then(devtools_dock_rect)) {
                (true, Some((x, y, w, h))) => {
                    SetWindowPos(
                        hwnd as _,
                        std::ptr::null_mut(),
                        x,
                        y,
                        w,
                        h,
                        SWP_NOACTIVATE | SWP_NOZORDER | SWP_SHOWWINDOW,
                    );
                }
                _ => {
                    ShowWindow(hwnd as _, SW_HIDE);
                }
            }
        }
    }
}

/// The developer tools finished loading: remember their window and paint them
/// in the browser's theme (colour tokens set on the root, which every panel
/// inherits).
pub fn devtools_loaded(browser: Option<&cef::Browser>, frame: &mut cef::Frame) {
    if devtools_anchor_opening() {
        // The anchor: stays hidden; now open the tools that were asked for.
        if let Some(host) = browser.and_then(|b| b.host()) {
            hide_devtools_window(host.window_handle().0 as isize);
        }
        DEVTOOLS_ANCHOR.store(2, std::sync::atomic::Ordering::Relaxed);
        let tab = DEVTOOLS_PENDING.lock().ok().and_then(|t| t.clone());
        let (x, y) = DEVTOOLS_POINT.lock().map(|p| *p).unwrap_or((0, 0));
        if let Some(host) = tab.as_deref().and_then(content_host) {
            dock_devtools(&host, x, y);
        }
        return;
    }
    if let Some(host) = browser.and_then(|b| b.host()) {
        let own = host.window_handle().0 as isize;
        let root = adopt_devtools_window(own);
        let tab = DEVTOOLS_PENDING
            .lock()
            .ok()
            .and_then(|mut t| t.take())
            .unwrap_or_else(active_tab_id);
        if let Ok(mut all) = DEVTOOLS.lock() {
            all.retain(|(t, hwnd, _)| t != &tab && *hwnd != root);
            all.push((tab, root, true));
        }
        super::app::relayout_registered_content();
    }
    // A close button in the top-right corner: the docked tools have no window
    // frame of their own. It asks AgentGo through the document title.
    const CLOSE: &str = r#"(()=>{if(document.getElementById('agentgo-close'))return;const b=document.createElement('button');b.id='agentgo-close';b.title='Close developer tools (F12)';b.setAttribute('aria-label','Close developer tools');b.innerHTML='<svg viewBox="0 0 10 10" width="10" height="10"><path d="M1.5 1.5l7 7m0-7-7 7" stroke="currentColor" stroke-width="1.4" stroke-linecap="round"/></svg>';Object.assign(b.style,{position:'fixed',top:'0',right:'0',width:'30px',height:'30px',display:'grid',placeItems:'center',border:'0',margin:'0',padding:'0',background:'var(--sys-color-cdt-base-container,#0F0F11)',color:'var(--sys-color-on-surface-subtle,#A49D95)',cursor:'pointer',zIndex:'2147483647'});b.onmouseenter=()=>{b.style.color='#fff';b.style.background='#e0402b'};b.onmouseleave=()=>{b.style.color='var(--sys-color-on-surface-subtle,#A49D95)';b.style.background='var(--sys-color-cdt-base-container,#0F0F11)'};const ask=()=>{document.title='agentgo:close-devtools';setTimeout(()=>{document.title='DevTools'},250)};b.onclick=ask;const put=()=>{if(!document.body)return setTimeout(put,50);document.body.style.setProperty('width','calc(100% - 30px)','important');document.body.appendChild(b)};put();document.addEventListener('keydown',e=>{if(e.key==='F12'){e.preventDefault();ask()}},true)})()"#;
    frame.execute_java_script(Some(&cef::CefString::from(CLOSE)), None, 0);
    let light = settings_snapshot()["theme"].as_str() == Some("light");
    let pairs: &[(&str, &str, &str)] = &[
        ("base", "#0A0A0B", "#F5F1EB"),
        ("base-container", "#0F0F11", "#EFEAE2"),
        ("base-container-elevated", "#1B1B1E", "#FFFFFF"),
        ("cdt-base-container", "#0F0F11", "#F5F1EB"),
        ("surface", "#0F0F11", "#F5F1EB"),
        ("surface1", "#141416", "#F2EDE6"),
        ("surface2", "#1B1B1E", "#EFEAE2"),
        ("surface3", "#232327", "#E8E1D8"),
        ("surface4", "#2A2A2F", "#E2DAD0"),
        ("surface5", "#313137", "#DAD1C6"),
        ("surface-variant", "#1B1B1E", "#E8E1D8"),
        ("neutral-container", "#1B1B1E", "#E8E1D8"),
        ("on-surface", "#F6F0E9", "#17110C"),
        ("on-surface-subtle", "#A49D95", "#6B6159"),
        ("on-surface-variant", "#A49D95", "#6B6159"),
        ("outline", "#3A3631", "#CFC5B9"),
        ("neutral-outline", "#3A3631", "#CFC5B9"),
        ("divider", "#26231F", "#DDD3C7"),
        ("primary", "#FF6A2B", "#F05A16"),
        ("on-primary", "#1A0D05", "#FFFFFF"),
        ("primary-container", "#3A1C0C", "#FFDCC9"),
        ("on-primary-container", "#FFD2B8", "#4A1E05"),
        ("tonal-container", "#2A1A10", "#FBE3D3"),
        ("secondary-container", "#2A1A10", "#FBE3D3"),
        ("on-secondary-container", "#FFD2B8", "#4A1E05"),
        ("tonal-outline", "#6B3A1D", "#E9B898"),
        ("state-focus-ring", "#FF6A2B", "#F05A16"),
        (
            "state-hover-on-subtle",
            "rgba(255,106,43,.10)",
            "rgba(240,90,22,.08)",
        ),
        (
            "state-hover-on-prominent",
            "rgba(255,255,255,.10)",
            "rgba(0,0,0,.08)",
        ),
        (
            "state-focus-highlight",
            "rgba(255,106,43,.20)",
            "rgba(240,90,22,.16)",
        ),
        ("inverse-surface", "#F6F0E9", "#17110C"),
        ("inverse-on-surface", "#0A0A0B", "#F5F1EB"),
    ];
    let vars: Vec<String> = pairs
        .iter()
        .map(|(name, dark, bright)| {
            format!(
                "['--sys-color-{name}','{}']",
                if light { bright } else { dark }
            )
        })
        .collect();
    let script = format!(
        "(()=>{{const v=[{}];const apply=()=>{{const s=document.documentElement.style;for(const [k,x] of v)s.setProperty(k,x,'important');s.setProperty('color-scheme','{}','important');}};apply();let n=0;const t=setInterval(()=>{{apply();if(++n>30)clearInterval(t);}},400);}})()",
        vars.join(","),
        if light { "light" } else { "dark" }
    );
    frame.execute_java_script(Some(&cef::CefString::from(script.as_str())), None, 0);
}

/// The panel colours DevTools should use: the browser's own theme.
fn devtools_theme() {
    let theme = match settings_snapshot()["theme"].as_str() {
        Some("light") => "default",
        Some("system") => "systemPreferred",
        _ => "dark",
    };
    if let Some(context) = content_context() {
        super::profiles::set_preference(
            &context,
            "devtools.preferences",
            &serde_json::json!({"uiTheme": format!("\"{theme}\"")}),
        );
    }
}

/// One choice from the right-click panel (UI thread).
pub fn context_action(params: &serde_json::Value) -> Result<serde_json::Value, String> {
    let action = params["action"].as_str().unwrap_or("");
    let tab = params["tabId"]
        .as_str()
        .map_or_else(active_tab_id, str::to_string);
    let x = params["x"].as_i64().unwrap_or(0) as i32;
    let y = params["y"].as_i64().unwrap_or(0) as i32;
    let frame = || {
        content_view_for_tab(&tab)
            .and_then(|view| view.browser())
            .and_then(|browser| browser.focused_frame().or_else(|| browser.main_frame()))
    };
    match action {
        "back" => {
            content_browser_action_for(&tab, BrowserAction::Back);
        }
        "forward" => {
            content_browser_action_for(&tab, BrowserAction::Forward);
        }
        "reload" => {
            content_browser_action_for(&tab, BrowserAction::Reload);
        }
        "cut" | "copy" | "paste" | "select_all" => {
            if let Some(frame) = frame() {
                match action {
                    "cut" => frame.cut(),
                    "copy" => frame.copy(),
                    "paste" => frame.paste(),
                    _ => frame.select_all(),
                }
            }
        }
        "translate" => {
            let text = params["selection"].as_str().unwrap_or("").to_string();
            let to = params["to"].as_str().unwrap_or("en").to_string();
            // The card opens at once, so the person sees it working instead of a jump later.
            show_context_report(serde_json::json!({
                "kind": "translate",
                "loading": true,
                "result": {"original": text, "to": to},
            }));
            super::exec::spawn(async move {
                let result = super::page::agent(
                    &tab,
                    "translateText",
                    serde_json::json!({"text": text, "to": to}),
                    15_000,
                )
                .await;
                let result = match result {
                    Ok(value) => serde_json::json!({
                        "original": text, "text": value["text"], "from": value["from"], "to": to,
                    }),
                    Err(error) => serde_json::json!({"error": error.message}),
                };
                show_context_report(serde_json::json!({"kind": "translate", "result": result}));
            });
        }
        "copy_translation" => clipboard_set_text(params["text"].as_str().unwrap_or("")),
        "copy_text" => {
            if let Some(frame) = frame() {
                frame.copy();
            }
        }
        "copy_link" | "copy_image" => {
            let key = if action == "copy_link" {
                "link"
            } else {
                "source"
            };
            clipboard_set_text(params[key].as_str().unwrap_or(""));
        }
        "open_link" | "open_image" => {
            let key = if action == "open_link" {
                "link"
            } else {
                "source"
            };
            let url = params[key].as_str().unwrap_or("");
            if url.starts_with("http://") || url.starts_with("https://") {
                create_content_tab(url)?;
            }
        }
        "print" => print_active_tab(),
        "inspect" => {
            devtools_theme();
            // Right-click Inspect names a point: inspect that element (in the
            // tools already there). F12 and the palette show or hide them.
            let at_point = params.get("x").is_some() && params.get("y").is_some();
            match devtools_of(&tab) {
                Some((_, shown)) if shown && !at_point => close_devtools(),
                Some(_) => {
                    set_devtools_shown(&tab, true);
                    if let Some(host) = content_host(&tab) {
                        if at_point {
                            dock_devtools(&host, x, y);
                        }
                    }
                    super::app::relayout_registered_content();
                }
                None => open_devtools(&tab, x, y),
            }
        }
        "a11y" => {
            super::exec::spawn(async move {
                let result = super::cdp::call(
                    &tab,
                    "Runtime.evaluate",
                    serde_json::json!({"expression": include_str!("ctx_audit.js"), "returnByValue": true}),
                )
                .await;
                let value = result.map_or_else(
                    |error| serde_json::json!({"error": error.to_string()}),
                    |out| out["result"]["value"].clone(),
                );
                show_context_report(serde_json::json!({"kind": "a11y", "result": value}));
            });
        }
        "fonts" => {
            super::exec::spawn(async move {
                let script = include_str!("ctx_fonts.js")
                    .replace("__X__", &x.to_string())
                    .replace("__Y__", &y.to_string());
                let result = super::cdp::call(
                    &tab,
                    "Runtime.evaluate",
                    serde_json::json!({"expression": script, "returnByValue": true}),
                )
                .await;
                let mut value = result.map_or_else(
                    |error| serde_json::json!({"error": error.to_string()}),
                    |out| out["result"]["value"].clone(),
                );
                // The fonts the engine really drew with, not just the CSS list.
                let drawn = async {
                    super::cdp::call(&tab, "DOM.enable", serde_json::json!({}))
                        .await
                        .ok()?;
                    super::cdp::call(&tab, "CSS.enable", serde_json::json!({}))
                        .await
                        .ok()?;
                    super::cdp::call(&tab, "DOM.getDocument", serde_json::json!({"depth": 0}))
                        .await
                        .ok()?;
                    let at = super::cdp::call(
                        &tab,
                        "DOM.getNodeForLocation",
                        serde_json::json!({"x": x, "y": y, "includeUserAgentShadowDOM": false}),
                    )
                    .await
                    .ok()?;
                    let node = super::cdp::call(
                        &tab,
                        "DOM.pushNodesByBackendIdsToFrontend",
                        serde_json::json!({"backendNodeIds": [at["backendNodeId"]]}),
                    )
                    .await
                    .ok()?;
                    let id = node["nodeIds"][0].clone();
                    super::cdp::call(
                        &tab,
                        "CSS.getPlatformFontsForNode",
                        serde_json::json!({"nodeId": id}),
                    )
                    .await
                    .ok()
                }
                .await;
                if let (Some(drawn), Some(map)) = (drawn, value.as_object_mut()) {
                    map.insert("drawn".into(), drawn["fonts"].clone());
                }
                show_context_report(serde_json::json!({"kind": "fonts", "result": value}));
            });
        }
        other => return Err(format!("unknown menu action {other}")),
    }
    Ok(serde_json::json!({"accepted": true}))
}

/// The zoom after one step from `now`: Chrome's zoom levels.
pub fn zoom_step(now: f64, step: &str) -> Option<f64> {
    const STOPS: [f64; 15] = [
        25.0, 33.0, 50.0, 67.0, 75.0, 80.0, 90.0, 100.0, 110.0, 125.0, 150.0, 175.0, 200.0, 300.0,
        500.0,
    ];
    match step {
        "in" => Some(
            STOPS
                .into_iter()
                .find(|stop| *stop > now + 0.5)
                .unwrap_or(500.0),
        ),
        "out" => Some(
            STOPS
                .into_iter()
                .rev()
                .find(|stop| *stop < now - 0.5)
                .unwrap_or(25.0),
        ),
        "reset" => Some(100.0),
        _ => None,
    }
}

/// Ctrl+Plus, Ctrl+Minus and Ctrl+0 on the visible tab.
fn zoom_shortcut(name: &str) {
    let tab = active_tab_id();
    let Ok(now) = zoom_tab(&tab, None) else {
        return;
    };
    if let Some(next) = zoom_step(now, name.trim_start_matches("zoom_")) {
        let _ = zoom_tab(&tab, Some(next));
    }
}

/// Runs a browser shortcut in the trusted chrome, which owns the one shortcut
/// table. Used when a page, not the chrome, has the keyboard focus, so a
/// shortcut behaves the same wherever the focus is.
pub fn shell_shortcut(name: &str) {
    if name.starts_with("zoom_") {
        return zoom_shortcut(name);
    }
    if name == "print" {
        return print_active_tab();
    }
    if name == "palette" {
        return open_tool_on_ui("palette");
    }
    if name == "find" {
        // Ctrl+F on an open find bar selects its text again.
        if current_tool().as_deref() == Some("find") {
            present_tool_on_ui("find");
        } else {
            open_tool_on_ui("find");
        }
        return;
    }
    let Some(view) = shell_view() else {
        return;
    };
    let Some(browser) = view.browser() else {
        return;
    };
    if name == "focus_address" {
        // The page's native view keeps the keyboard until focus is moved.
        if let Some(host) = browser.host() {
            host.set_focus(1);
        }
    }
    let (Ok(argument), Some(frame)) = (serde_json::to_string(name), browser.main_frame()) else {
        return;
    };
    frame.execute_java_script(
        Some(&cef::CefString::from(
            format!("window.__agentgoShortcut && window.__agentgoShortcut({argument});").as_str(),
        )),
        Some(&cef::CefString::from("agentgo://shell/chrome.html")),
        0,
    );
}

/// Applies the trusted shell's CSS draggable regions to the frameless native
/// window. CEF copies the region list during this call; the borrowed callback
/// slice never escapes the UI-thread callback.
pub fn set_shell_draggable_regions(regions: Option<&[cef::DraggableRegion]>) {
    let window = state()
        .window
        .lock()
        .ok()
        .and_then(|slot| slot.as_ref().map(|handle| handle.0.clone()));
    if let Some(window) = window {
        window.set_draggable_regions(regions);
    }
}

/// Runs transport jobs and pending command tasks. Called only on the CEF UI
/// thread: from the external pump loop and from the desktop tick task.
pub fn drain_protocol_requests() -> usize {
    super::exec::tick()
}

/// Returns the current non-secret settings snapshot.
pub fn settings_snapshot() -> serde_json::Value {
    let Some(app) = STATE.get() else {
        return default_settings();
    };
    app.settings
        .lock()
        .map(|value| value.clone())
        .unwrap_or_else(|_| default_settings())
}

/// Merges and persists an allowlisted settings patch. Secrets are never
/// accepted here; credentials belong to the OS-backed profile store that is
/// not exposed through this generic settings endpoint.
pub fn update_settings(patch: &serde_json::Value) -> Result<serde_json::Value, String> {
    let Some(patch) = patch.as_object() else {
        return Err("settings.set requires an object in params.settings".into());
    };
    for key in patch.keys() {
        let lower = key.to_ascii_lowercase();
        if lower.contains("pass")
            || lower.contains("secret")
            || lower.contains("token")
            || lower.contains("cookie")
            || lower.contains("credential")
        {
            return Err(format!(
                "setting {key} cannot contain credentials or secrets"
            ));
        }
    }

    let mut guard = state()
        .settings
        .lock()
        .map_err(|_| "settings lock is unavailable".to_string())?;
    if !guard.is_object() {
        *guard = default_settings();
    }
    {
        let current = guard
            .as_object_mut()
            .ok_or_else(|| "settings defaults are invalid".to_string())?;
        for (key, value) in patch {
            current.insert(key.clone(), value.clone());
        }
    }
    let snapshot = guard.clone();
    let store = AtomicStateStore::new(&state().settings_path, vec![])
        .map_err(|error| format!("settings store unavailable: {error}"))?;
    store
        .save(&snapshot)
        .map_err(|error| format!("settings save failed: {error}"))?;
    drop(guard);
    super::adblock::apply(&snapshot);
    super::vpn::apply(&snapshot);
    super::sitedark::refresh(&snapshot);
    if patch.contains_key("fingerprintProtection") {
        super::exec::post(super::fingerprint::refresh_all);
    }
    if patch.contains_key("blockThirdPartyCookies") {
        super::exec::post(|| {
            if let Some(context) = content_context() {
                super::profiles::apply_cookie_policy(&context);
            }
        });
    }
    apply_chrome_height();
    push_theme(&snapshot);
    Ok(snapshot)
}

thread_local! {
    /// Where the person clicked to change the theme, in window coordinates.
    static THEME_ORIGIN: std::cell::Cell<Option<(i32, i32)>> = const { std::cell::Cell::new(None) };
}

/// The panel reports the point of the click that is about to change the
/// theme, in its own coordinates.
pub fn set_theme_origin(x: i32, y: i32) {
    // where AgentGo itself placed the panel (a view inside an overlay does
    // not report its own position on screen reliably)
    let offset = TOOL_RECT.with(std::cell::Cell::get);
    THEME_ORIGIN.with(|origin| origin.set(Some((x + offset.0, y + offset.1))));
}

/// Where a view sits relative to the toolbar's top-left corner (the window's
/// content origin), from both views' positions on screen. An overlay's own
/// bounds are not window coordinates, so they cannot be used for this.
fn offset_of(view: Option<cef::BrowserView>) -> (i32, i32) {
    let (Some(view), Some(chrome)) = (view, shell_view()) else {
        return (0, 0);
    };
    let (a, b) = (view.bounds_in_screen(), chrome.bounds_in_screen());
    (a.x - b.x, a.y - b.y)
}

/// Runs a script in the toolbar document (UI thread).
pub fn run_in_shell(script: &str) {
    run_in(shell_view(), script);
}

fn run_in(view: Option<cef::BrowserView>, script: &str) {
    if let Some(frame) = view
        .and_then(|view| view.browser())
        .and_then(|browser| browser.main_frame())
    {
        frame.execute_java_script(Some(&cef::CefString::from(script)), None, 0);
    }
}

/// Tells the chrome, the panels and the pages about new settings. A theme
/// change is sent to all of them at the same moment with one origin and one
/// radius (each in its own coordinates), so they play a single circle
/// across the window.
fn push_theme(settings: &serde_json::Value) {
    let window = state()
        .window
        .lock()
        .ok()
        .and_then(|slot| slot.as_ref().map(|handle| handle.0.clone()));
    if let Some(window) = window {
        let size = window.bounds();
        let (x, y) = THEME_ORIGIN
            .with(|origin| origin.take())
            .unwrap_or((size.width - 60, 60));
        let far = |cx: i32, cy: i32| (f64::from(cx - x).powi(2) + f64::from(cy - y).powi(2)).sqrt();
        let r = far(0, 0)
            .max(far(size.width, 0))
            .max(far(0, size.height))
            .max(far(size.width, size.height))
            .ceil()
            + 2.0;
        let theme = settings["theme"].as_str().unwrap_or("dark");
        let sync = |dx: i32, dy: i32| {
            format!(
                "window.__agentgoThemeSync && window.__agentgoThemeSync({});",
                serde_json::json!({"theme": theme, "x": x - dx, "y": y - dy, "r": r})
            )
        };
        run_in(shell_view(), &sync(0, 0));
        let (dx, dy) = TOOL_RECT.with(std::cell::Cell::get);
        run_in(tool_view(), &sync(dx, dy));
        for id in content_tab_ids() {
            let view = content_view_for_tab(&id);
            let (dx, dy) = offset_of(view.clone());
            run_in(view, &sync(dx, dy));
        }
    }
    run_in(
        shell_view(),
        &format!("window.__agentgoTheme && window.__agentgoTheme({settings});"),
    );
}

/// Whether the operator turned hardware acceleration off in settings. Read
/// before CEF starts; it takes effect on the next launch.
pub fn software_rendering_requested(options: &RunOptions) -> bool {
    let (_, settings_path, _) = persistent_paths(&options.cache_root);
    load_settings(&settings_path)
        .get("hardwareAcceleration")
        .and_then(|value| value.as_bool())
        == Some(false)
}

/// Settings -> Languages: the Accept-Language list pages see, for example
/// "en-US,en". Read before CEF starts; invalid values are ignored.
pub fn startup_languages(options: &RunOptions) -> Option<String> {
    let (_, settings_path, _) = persistent_paths(&options.cache_root);
    let list = load_settings(&settings_path)
        .get("languages")?
        .as_str()?
        .replace(' ', "");
    let valid = !list.is_empty()
        && list.len() <= 120
        && list.split(',').all(|tag| {
            !tag.is_empty() && tag.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        });
    valid.then_some(list)
}

/// Returns the newest bounded history entries.
pub fn history_snapshot(limit: usize) -> Vec<serde_json::Value> {
    state()
        .history
        .lock()
        .map(|items| items.iter().take(limit.clamp(1, 100)).cloned().collect())
        .unwrap_or_default()
}

/// Deletes matching history entries from the mirror and the durable table.
pub fn delete_history(
    url: Option<&str>,
    contains: Option<&str>,
    since: Option<u64>,
) -> Result<usize, String> {
    if let Ok(mut items) = state().history.lock() {
        items.retain(|item| {
            let item_url = item["url"].as_str().unwrap_or("");
            let text =
                format!("{} {}", item_url, item["title"].as_str().unwrap_or("")).to_lowercase();
            !(url == Some(item_url)
                || contains
                    .is_some_and(|part| !part.is_empty() && text.contains(&part.to_lowercase()))
                || since.is_some_and(|since| item["visitedAt"].as_u64().unwrap_or(0) >= since))
        });
    }
    with_store(|store| store.delete_history(url, contains, since))
}

/// Clears this instance's history mirror and the durable SQLite history table.
/// The operation remains deliberately explicit and is never implied by a new
/// browser process.
pub fn clear_history() {
    if let Ok(mut items) = state().history.lock() {
        items.clear();
    }
    if let Ok(store) = SqliteStateStore::open(&state().history_path) {
        let _ = store.clear_history();
    }
}

/// Whether CEF downloads are allowed for this run. Downloads remain disabled
/// by default until the operator enables the non-secret setting explicitly.
pub fn downloads_enabled() -> bool {
    state().options.allow_downloads
        || state()
            .settings
            .lock()
            .ok()
            .and_then(|settings| settings.get("downloadsEnabled").and_then(|v| v.as_bool()))
            .unwrap_or(false)
}

/// Returns a safe, scoped destination for one download.
pub fn download_destination(id: u32, suggested_name: &str) -> PathBuf {
    let mut name: String = suggested_name
        .trim()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_' | ' ') {
                ch
            } else {
                '_'
            }
        })
        .collect();
    if name.is_empty() || name == "." || name == ".." {
        name = format!("download-{id}.bin");
    }
    let name = name.trim_matches(['.', ' ']);
    let name = if name.is_empty() {
        format!("download-{id}.bin")
    } else {
        name.chars().take(160).collect()
    };
    let dir = state()
        .download_dir
        .lock()
        .map(|dir| dir.clone())
        .unwrap_or_default();
    // A file of the same name is never overwritten: "name (1).ext", as browsers do.
    let path = dir.join(&name);
    if !path.exists() {
        return path;
    }
    let (stem, extension) = match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => (stem.to_string(), format!(".{extension}")),
        _ => (name.clone(), String::new()),
    };
    (1..1000)
        .map(|n| dir.join(format!("{stem} ({n}){extension}")))
        .find(|candidate| !candidate.exists())
        .unwrap_or_else(|| dir.join(format!("{stem}-{id}{extension}")))
}

/// The scoped download folder of this profile.
pub fn download_folder() -> PathBuf {
    state()
        .download_dir
        .lock()
        .map(|dir| dir.clone())
        .unwrap_or_default()
}

/// Settings -> Downloads -> Open folder: shows the scoped download folder in
/// File Explorer. Nothing in it is opened.
pub fn open_download_folder() -> Result<serde_json::Value, String> {
    let dir = state()
        .download_dir
        .lock()
        .map(|dir| dir.clone())
        .map_err(|_| "download folder is unavailable".to_string())?;
    std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    std::process::Command::new("explorer.exe")
        .arg(&dir)
        .spawn()
        .map_err(|error| format!("File Explorer did not start: {error}"))?;
    Ok(serde_json::json!({"path": dir.to_string_lossy()}))
}

/// Re-reads the visible history list from disk (after an import).
pub fn reload_history() {
    let fresh = load_history(&state().history_path);
    if let Ok(mut items) = state().history.lock() {
        *items = fresh;
    }
}

/// The folder downloads (and saved problem reports) go to.
pub fn download_dir() -> std::path::PathBuf {
    state()
        .download_dir
        .lock()
        .map(|dir| dir.clone())
        .unwrap_or_else(|_| std::env::temp_dir())
}

/// The last `count` lines of the browser's own event log.
pub fn recent_events(count: usize) -> Vec<String> {
    state().events.lock().map_or_else(
        |_| Vec::new(),
        |events| events[events.len().saturating_sub(count)..].to_vec(),
    )
}

/// Records the CEF item before it starts writing bytes.
pub fn record_download_before(item: &DownloadItem, suggested_name: &str, path: &std::path::Path) {
    let value = download_value(item, suggested_name, path);
    if let Ok(mut downloads) = state().downloads.lock() {
        let id = item.id();
        downloads.retain(|entry| entry.get("id").and_then(|v| v.as_u64()) != Some(id as u64));
        downloads.insert(0, value);
        downloads.truncate(100);
        super::misc::note_downloads(&downloads);
    }
    log_event(&format!("AG-RUNTIME download-accepted id={}", item.id()));
}

/// Refreshes the public metadata snapshot after a download progress event.
pub fn record_download_update(item: &DownloadItem) {
    let mut to_scan = None;
    if let Ok(mut downloads) = state().downloads.lock() {
        let id = item.id();
        if let Some(entry) = downloads
            .iter_mut()
            .find(|entry| entry.get("id").and_then(|v| v.as_u64()) == Some(id as u64))
        {
            let suggested = entry
                .get("suggestedName")
                .and_then(|v| v.as_str())
                .unwrap_or("download.bin")
                .to_string();
            let path = entry
                .get("path")
                .and_then(|v| v.as_str())
                .map(PathBuf::from)
                .unwrap_or_else(|| download_destination(id, &suggested));
            let scan = entry.get("scan").cloned();
            *entry = download_value(item, &suggested, &path);
            if let Some(scan) = scan {
                entry["scan"] = scan;
            } else if entry["status"] == "complete" {
                entry["scan"] = serde_json::json!({"status": "scanning"});
                let text = |key: &str| entry[key].as_str().unwrap_or("").to_string();
                let (url, original) = (text("url"), text("originalUrl"));
                let referrer = if original != url {
                    original
                } else {
                    String::new()
                };
                to_scan = Some((id as u64, path, url, referrer));
            }
        }
        super::misc::note_downloads(&downloads);
    }
    if let Some((id, path, url, referrer)) = to_scan {
        std::thread::spawn(move || {
            let scan = super::scan::scan_file(&path, &url, &referrer);
            log_event(&format!(
                "AG-RUNTIME download-scan id={id} status={}",
                scan["status"].as_str().unwrap_or("?")
            ));
            if let Ok(mut downloads) = state().downloads.lock() {
                if let Some(entry) = downloads
                    .iter_mut()
                    .find(|entry| entry.get("id").and_then(|v| v.as_u64()) == Some(id))
                {
                    entry["scan"] = scan;
                }
            }
        });
    }
}

/// The scan result recorded for a downloaded file, if any.
pub fn download_scan_status(path: &str) -> Option<String> {
    let wanted = std::path::Path::new(path).canonicalize().ok()?;
    downloads_snapshot().into_iter().find_map(|d| {
        let same = d["path"]
            .as_str()
            .and_then(|p| std::path::Path::new(p).canonicalize().ok())
            .is_some_and(|p| p == wanted);
        same.then(|| d["scan"]["status"].as_str().map(str::to_string))
            .flatten()
    })
}

/// Takes a finished download out of the list (the file stays).
pub fn remove_download(id: u64) -> Result<bool, String> {
    let mut downloads = state()
        .downloads
        .lock()
        .map_err(|_| "downloads are unavailable".to_string())?;
    if downloads
        .iter()
        .any(|d| d["id"].as_u64() == Some(id) && d["inProgress"] == true)
    {
        return Err("the download is still running; download.cancel first".into());
    }
    let before = downloads.len();
    downloads.retain(|d| d["id"].as_u64() != Some(id));
    Ok(downloads.len() < before)
}

/// Retains the current UI-thread download callback so the agent can request a
/// native pause, resume, or cancel while the transfer is active. CEF may omit
/// the callback for completed items; those items remain metadata-only.
pub fn remember_download_callback(id: u32, callback: &DownloadItemCallback) {
    DOWNLOAD_CALLBACKS.with(|callbacks| {
        let mut callbacks = callbacks.borrow_mut();
        callbacks.insert(id, callback.clone());
        // A download record list is bounded to 100, so keeping a little extra
        // callback headroom is enough for updates without allowing a page to
        // grow native callback state indefinitely.
        while callbacks.len() > 128 {
            let Some(oldest) = callbacks.keys().next().copied() else {
                break;
            };
            callbacks.remove(&oldest);
        }
    });
}

fn download_value(
    item: &DownloadItem,
    suggested_name: &str,
    path: &std::path::Path,
) -> serde_json::Value {
    let status = if item.is_canceled() != 0 {
        "canceled"
    } else if item.is_interrupted() != 0 {
        "interrupted"
    } else if item.is_complete() != 0 {
        "complete"
    } else {
        "in_progress"
    };
    serde_json::json!({
        "id": item.id(),
        "url": CefString::from(&item.url()).to_string(),
        "originalUrl": CefString::from(&item.original_url()).to_string(),
        "suggestedName": suggested_name,
        "path": path.to_string_lossy(),
        "status": status,
        "inProgress": item.is_in_progress() != 0,
        "complete": item.is_complete() != 0,
        "canceled": item.is_canceled() != 0,
        "receivedBytes": item.received_bytes(),
        "totalBytes": item.total_bytes(),
        "percent": item.percent_complete()
        ,"paused": item.is_paused() != 0
    })
}

/// Lists metadata only; file contents are never sent to the agent.
pub fn downloads_snapshot() -> Vec<serde_json::Value> {
    state()
        .downloads
        .lock()
        .map(|items| items.clone())
        .unwrap_or_default()
}

/// Starts a download in one active content tab. Downloads must be enabled by
/// the operator first; a protocol caller cannot silently turn the gate on.
pub fn start_download(params: &serde_json::Value) -> Result<serde_json::Value, String> {
    if !downloads_enabled() {
        return Err("downloads are disabled; enable settings.downloadsEnabled first".into());
    }
    let tab_id = requested_tab_id(params)?;
    let url = params
        .get("url")
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .or_else(|| {
            content_tab_snapshot_for(&tab_id).and_then(|snapshot| {
                snapshot
                    .get("url")
                    .and_then(|value| value.as_str())
                    .map(str::to_string)
            })
        })
        .ok_or_else(|| "download.start requires params.url or a loaded tab".to_string())?;
    if !is_navigable_url(&url) {
        return Err("download URL must use http, https, file, or about".into());
    }
    let host = content_browser_host_for_tab(&tab_id)
        .ok_or_else(|| "content browser is not ready".to_string())?;
    host.start_download(Some(&CefString::from(url.as_str())));
    Ok(serde_json::json!({"accepted":true,"tabId":tab_id,"url":url}))
}

/// Returns progress metadata for one download id or all current downloads.
pub fn download_progress(params: &serde_json::Value) -> Result<serde_json::Value, String> {
    let items = downloads_snapshot();
    let Some(id) = params.get("id").and_then(|value| value.as_u64()) else {
        return Ok(serde_json::json!({"items":items}));
    };
    let item = items
        .into_iter()
        .find(|item| item.get("id").and_then(|value| value.as_u64()) == Some(id))
        .ok_or_else(|| format!("unknown download id {id}"))?;
    Ok(serde_json::json!({"item":item}))
}

/// Applies one native control to an active download. The operation never
/// opens a file or executes downloaded content; it only forwards the action
/// to CEF's current UI-thread callback.
pub fn control_download(
    method: &str,
    params: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let id = params
        .get("id")
        .and_then(|value| value.as_u64())
        .ok_or_else(|| format!("{method} requires integer params.id"))?;
    let id = u32::try_from(id).map_err(|_| "download id exceeds u32".to_string())?;
    let downloads = downloads_snapshot();
    let Some(item) = downloads
        .iter()
        .find(|item| item.get("id").and_then(|value| value.as_u64()) == Some(id as u64))
    else {
        return Err(format!("unknown download id {id}"));
    };
    if item.get("inProgress").and_then(|value| value.as_bool()) != Some(true) {
        return Err(format!("download {id} is no longer active"));
    }
    let action = method.strip_prefix("download.").unwrap_or(method);
    let applied = DOWNLOAD_CALLBACKS.with(|callbacks| {
        let callbacks = callbacks.borrow();
        let Some(callback) = callbacks.get(&id) else {
            return false;
        };
        match action {
            "pause" => callback.pause(),
            "resume" => callback.resume(),
            "cancel" => callback.cancel(),
            _ => return false,
        }
        true
    });
    if !applied {
        return Err(format!("download {id} is no longer active"));
    }
    log_event(&format!("AG-RUNTIME download-{action} id={id}"));
    Ok(serde_json::json!({"accepted":true,"id":id,"action":action}))
}

/// Creates and attaches a new native content tab in the shared isolated
/// request context. New tabs become active immediately, matching normal
/// browser behavior while keeping the shell view above every page.
pub fn create_content_tab(url: &str) -> Result<serde_json::Value, String> {
    create_content_tab_placed(url, Placement::Front)
}

/// [`create_content_tab`] with an explicit choice of whether the new tab
/// replaces the visible one.
pub fn create_content_tab_placed(
    url: &str,
    placement: Placement,
) -> Result<serde_json::Value, String> {
    if !is_navigable_url(url) {
        return Err("url must use http, https, file, or about".into());
    }
    let id = allocate_tab_id();
    create_content_tab_with_id(&id, url, placement)
}

/// Whether the first content tab exists, i.e. commands can be served.
pub fn ready() -> bool {
    !content_tab_ids().is_empty()
}

/// Creates the first tab on the start page. In smoke mode the scenario
/// state machine drives the first navigation instead.
pub fn create_first_tab() -> Result<(), String> {
    let smoke = state().options.smoke;
    // A launch that names no page continues where the last one stopped: a
    // person's own launch unless Settings says "On startup: new tab", an
    // agent's launch only when asked.
    let options = &state().options;
    let person_restores = !options.stdio && settings_snapshot()["startup"] != "newtab";
    let restore = !smoke && options.content_url.is_empty() && (options.restore || person_restores);
    let mut session = if restore { saved_session() } else { Vec::new() }.into_iter();
    let url = if smoke {
        "about:blank".to_string()
    } else {
        session
            .next()
            .unwrap_or_else(|| options.content_start_url())
    };
    create_content_tab_with_id("content-1", &url, Placement::Front)?;
    for url in session {
        create_content_tab_with_id(&allocate_tab_id(), &url, Placement::Background)?;
    }
    if smoke
        && state()
            .smoke_step
            .lock()
            .is_ok_and(|step| *step == SmokeStep::AwaitContent)
    {
        // The shell finished loading before the profile was ready.
        navigate_content(&state().options.content_start_url());
    }
    Ok(())
}

/// A new tab in another request context (a side profile).
pub fn create_content_tab_in(
    url: &str,
    placement: Placement,
    context: cef::RequestContext,
) -> Result<serde_json::Value, String> {
    if !is_navigable_url(url) {
        return Err("url must use http, https, file, or about".into());
    }
    create_content_tab_in_context(&allocate_tab_id(), url, placement, Some(context))
}

fn create_content_tab_with_id(
    id: &str,
    url: &str,
    placement: Placement,
) -> Result<serde_json::Value, String> {
    create_content_tab_in_context(id, url, placement, None)
}

fn create_content_tab_in_context(
    id: &str,
    url: &str,
    placement: Placement,
    context: Option<cef::RequestContext>,
) -> Result<serde_json::Value, String> {
    let mut context = context
        .or_else(content_context)
        .ok_or_else(|| "content context is not ready".to_string())?;
    let view = super::app::create_content_browser_view(id, url, &mut context)
        .ok_or_else(|| "CEF could not create the content tab".to_string())?;
    let host = tab_host().ok_or_else(|| "composition window is not ready".to_string())?;
    // Register before attaching: CEF fires on_browser_created during
    // add_child_view and that callback looks the tab up by id.
    register_content_tab(id, view.clone())?;
    let mut as_view = View::from(&view);
    host.add_child_view(Some(&mut as_view));
    if placement == Placement::Front {
        if let Ok(mut active) = state().active_tab_id.lock() {
            *active = id.to_string();
        }
    }
    super::app::relayout_registered_content();
    // The utility overlay must be created after the newest child View to keep
    // CEF's documented high z-order guarantee for every tab.
    restack_tool_overlay()?;
    notify_shell_state();
    Ok(serde_json::json!({
        "tabId": id,
        "active": placement == Placement::Front,
        "accepted": true,
        "url": url
    }))
}

/// Closes a native content tab after detaching it from the composition
/// window. The final remaining tab is protected by queue_close_content_tab.
pub fn close_content_tab(id: &str) -> Result<serde_json::Value, String> {
    let (view, next_active) = remove_content_tab(id)?;
    let released = super::cdp::detach(id);
    super::sessions::forget_tab(id);
    // A tab in another window leaves that window.
    if let Some(host) = super::winmgr::forget_tab(id).or_else(tab_host) {
        let mut as_view = View::from(&view);
        host.remove_child_view(Some(&mut as_view));
    }
    if let Some(host) = view.browser().and_then(|browser| browser.host()) {
        // The tab is gone from the window at once; its browser closes as soon
        // as the page has let go of its emulated focus.
        super::exec::spawn(async move {
            released.await;
            host.close_browser(1);
        });
    }
    super::app::relayout_registered_content();
    if let Some(host) = content_browser_host_for_tab(&next_active) {
        focus_page(&host);
    }
    notify_shell_state();
    Ok(serde_json::json!({
        "tabId": id,
        "closed": true,
        "activeTabId": next_active
    }))
}

/// Removes finished/canceled records while preserving active transfers.
pub fn clear_downloads() {
    if let Ok(mut downloads) = state().downloads.lock() {
        downloads.retain(|item| item.get("inProgress").and_then(|v| v.as_bool()) == Some(true));
    }
}

/// The folder downloads go to.
pub fn download_root() -> PathBuf {
    state()
        .download_dir
        .lock()
        .map(|dir| dir.clone())
        .unwrap_or_default()
}

/// The browser host of a content tab.
pub fn content_host(id: &str) -> Option<BrowserHost> {
    content_browser_host_for_tab(id)
}

fn content_browser_host_for_tab(id: &str) -> Option<BrowserHost> {
    content_view_for_tab(id)
        .and_then(|view| view.browser())
        .and_then(|browser| browser.host())
}

/// The main AgentGo window.
pub fn main_window() -> Option<cef::Window> {
    state()
        .window
        .lock()
        .ok()
        .and_then(|slot| slot.as_ref().map(|handle| handle.0.clone()))
}

/// Native window controls executed on the CEF/UI thread.
#[derive(Debug, Clone, Copy)]
pub enum WindowAction {
    /// Minimize the top-level window.
    Minimize,
    /// Maximize or restore the top-level window.
    Maximize,
    /// Leave the maximized or minimized state.
    Restore,
    /// Enter (true) or leave (false) full screen.
    Fullscreen(bool),
    /// Keep the window above others (true) or not (false).
    AlwaysOnTop(bool),
}

/// Applies a native window action and returns a post-action state snapshot.
pub fn window_action(action: WindowAction) -> Option<serde_json::Value> {
    let window = state()
        .window
        .lock()
        .ok()
        .and_then(|slot| slot.as_ref().map(|handle| handle.0.clone()))?;
    match action {
        WindowAction::Minimize => window.minimize(),
        WindowAction::Maximize => {
            if window.is_maximized() != 0 {
                window.restore();
            } else {
                window.maximize();
            }
        }
        WindowAction::Restore => window.restore(),
        WindowAction::Fullscreen(on) => window.set_fullscreen(on as i32),
        WindowAction::AlwaysOnTop(on) => window.set_always_on_top(on as i32),
    }
    Some(serde_json::json!({
        "minimized": window.is_minimized() != 0,
        "maximized": window.is_maximized() != 0,
        "fullscreen": window.is_fullscreen() != 0,
        "alwaysOnTop": window.is_always_on_top() != 0,
        "active": window.is_active() != 0
    }))
}

/// The window's state without changing it.
fn window_action_state() -> Option<serde_json::Value> {
    let window = state()
        .window
        .lock()
        .ok()
        .and_then(|slot| slot.as_ref().map(|handle| handle.0.clone()))?;
    Some(serde_json::json!({"fullscreen": window.is_fullscreen() != 0}))
}

/// Mutes or unmutes the sound of a tab.
pub fn mute_tab(tab_id: &str, muted: bool) -> Result<bool, String> {
    let host = content_browser_host_for_tab(tab_id)
        .ok_or_else(|| "content browser is not ready".to_string())?;
    host.set_audio_muted(muted as i32);
    Ok(host.is_audio_muted() != 0)
}

/// Resizes the native AgentGo window. Width and height are intentionally
/// bounded so an agent cannot request an invalid or effectively unbounded
/// native surface; the window delegate relayouts both BrowserViews.
pub fn window_resize(params: &serde_json::Value) -> Result<serde_json::Value, String> {
    let width = params
        .get("width")
        .and_then(|value| value.as_i64())
        .ok_or_else(|| "window.resize requires integer params.width".to_string())?;
    let height = params
        .get("height")
        .and_then(|value| value.as_i64())
        .ok_or_else(|| "window.resize requires integer params.height".to_string())?;
    if !(320..=10_000).contains(&width) || !(240..=10_000).contains(&height) {
        return Err("window dimensions must be width 320..10000 and height 240..10000".into());
    }
    let window = state()
        .window
        .lock()
        .ok()
        .and_then(|slot| slot.as_ref().map(|handle| handle.0.clone()))
        .ok_or_else(|| "browser window is not ready".to_string())?;
    let current = window.bounds();
    window.set_bounds(Some(&Rect {
        x: current.x,
        y: current.y,
        width: width as i32,
        height: height as i32,
    }));
    Ok(serde_json::json!({
        "accepted": true,
        "width": width,
        "height": height,
        "minimized": window.is_minimized() != 0,
        "maximized": window.is_maximized() != 0,
        "fullscreen": window.is_fullscreen() != 0,
        "active": window.is_active() != 0
    }))
}

fn profile_catalog() -> Result<ProfileCatalog, String> {
    let root = state()
        .settings_path
        .parent()
        .and_then(|config| config.parent())
        .ok_or_else(|| "profile root is unavailable".to_string())?;
    ProfileCatalog::open(AgentGoPaths::with_root(root)).map_err(|error| error.to_string())
}

/// Lists profile metadata without exposing cookies or credentials.
pub fn profiles_list() -> Result<Vec<agentgo_storage::ProfileRecord>, String> {
    profile_catalog().and_then(|catalog| catalog.list().map_err(|error| error.to_string()))
}

/// Creates a durable profile record and its scoped directory.
pub fn profile_create(
    id: &str,
    name: &str,
    ephemeral: bool,
) -> Result<agentgo_storage::ProfileRecord, String> {
    profile_catalog().and_then(|catalog| {
        catalog
            .create(id, name, ephemeral)
            .map_err(|error| error.to_string())
    })
}

/// Deletes profile metadata only after an explicit confirmation.
pub fn profile_delete(id: &str, confirm: bool) -> Result<(), String> {
    if id == active_profile_id() {
        return Err("the active profile cannot be deleted; launch another profile first".into());
    }
    profile_catalog().and_then(|catalog| {
        catalog
            .delete(id, confirm)
            .map_err(|error| error.to_string())
    })
}

pub(crate) fn handle_protocol_request(request: Request) -> Response {
    if request.v != agentgo_protocol::envelope::ENVELOPE_VERSION {
        return Response::err(
            request.id,
            ErrorCode::InvalidEnvelope,
            format!("unsupported envelope version {}", request.v),
        );
    }

    match request.method.as_str() {
        "api.health" | "browser.health" => Response::ok(request.id, {
            let crash_reports = state()
                .crash_reports
                .lock()
                .map(|reports| reports.len())
                .unwrap_or(0);
            let unresponsive_tabs = state()
                .content_tabs
                .lock()
                .map(|tabs| tabs.iter().filter(|tab| tab.unresponsive).count())
                .unwrap_or(0);
            serde_json::json!({
            "status": "ready",
            "browserProcess": true,
            "shell": shell_view().is_some(),
            "content": content_view().is_some(),
            "window": state().window.lock().map(|w| w.is_some()).unwrap_or(false),
            "externalMessagePump": true,
            "gpu": render_mode(),
            "instanceId": state().instance_id,
            "profileId": active_profile_id(),
            "transport": if state().options.stdio { "stdio" } else { "desktop" },
            "endpointRecord": state().endpoint_store.is_some(),
            "rendererCrashReports": crash_reports,
            "unresponsiveTabs": unresponsive_tabs,
            "permissionPolicy": "default-deny",
            "nativeFileDialog": "cef-windows-handoff",
            "profile": profile_info(),
            "pipe": super::pipe::pipe_name(&state().instance_id),
            "agentSessions": super::sessions::session_count(),
            "controlMode": control_mode()
            })
        }),
        "api.version" | "browser.version" => Response::ok(
            request.id,
            serde_json::json!({
                "agentgo": env!("CARGO_PKG_VERSION"),
                "protocol": agentgo_protocol::PROTOCOL_VERSION,
                "protocolMin": agentgo_protocol::MIN_SUPPORTED_PROTOCOL_VERSION,
                "skill": agentgo_protocol::SKILL_VERSION,
                "cef": crate::CEF_VERSION,
                "chromium": crate::CEF_CHROMIUM_VERSION,
                "cefRsRevision": crate::CEF_RS_REVISION
            }),
        ),
        "profile.list" => match profiles_list() {
            Ok(profiles) => Response::ok(request.id, serde_json::json!({"profiles":profiles})),
            Err(message) => Response::err(request.id, ErrorCode::Internal, message),
        },
        "profile.create" => {
            let id = request.params.get("id").and_then(|value| value.as_str());
            let Some(id) = id else {
                return Response::err(
                    request.id,
                    ErrorCode::InvalidParams,
                    "profile.create requires params.id",
                );
            };
            let name = request
                .params
                .get("name")
                .and_then(|value| value.as_str())
                .unwrap_or(id);
            let ephemeral = request
                .params
                .get("ephemeral")
                .and_then(|value| value.as_bool())
                .unwrap_or(false);
            match profile_create(id, name, ephemeral) {
                Ok(profile) => Response::ok(request.id, serde_json::json!({"profile":profile})),
                Err(message) => Response::err(request.id, ErrorCode::InvalidParams, message),
            }
        }
        "profile.delete" => {
            let id = request.params.get("id").and_then(|value| value.as_str());
            let Some(id) = id else {
                return Response::err(
                    request.id,
                    ErrorCode::InvalidParams,
                    "profile.delete requires params.id",
                );
            };
            let confirm = request
                .params
                .get("confirm")
                .and_then(|value| value.as_bool())
                .unwrap_or(false);
            match profile_delete(id, confirm) {
                Ok(()) => Response::ok(request.id, serde_json::json!({"deleted":true})),
                Err(message) if !confirm => {
                    Response::err(request.id, ErrorCode::ApprovalRequired, message)
                }
                Err(message) => Response::err(request.id, ErrorCode::InvalidParams, message),
            }
        }
        "settings.get" => {
            let mut settings = settings_snapshot();
            super::adblock::decorate(&mut settings);
            super::vpn::decorate(&mut settings);
            Response::ok(request.id, settings)
        }
        "settings.set" => {
            let patch = request.params.get("settings").unwrap_or(&request.params);
            match update_settings(patch) {
                Ok(settings) => Response::ok(request.id, settings),
                Err(message) => Response::err(request.id, ErrorCode::InvalidParams, message),
            }
        }
        "history.list" => {
            let limit = request
                .params
                .get("limit")
                .and_then(|value| value.as_u64())
                .unwrap_or(50) as usize;
            let query = request.params["query"]
                .as_str()
                .unwrap_or("")
                .to_lowercase();
            let items: Vec<serde_json::Value> =
                history_snapshot(if query.is_empty() { limit } else { 100 })
                    .into_iter()
                    .filter(|item| query.is_empty() || matches_query(item, &query))
                    .take(limit.max(1))
                    .collect();
            Response::ok(request.id, serde_json::json!({"items": items}))
        }
        "bookmark.list" => match with_store(|store| store.bookmarks(200)) {
            Ok(items) => {
                let query = request.params["query"]
                    .as_str()
                    .unwrap_or("")
                    .to_lowercase();
                let items: Vec<serde_json::Value> = items
                    .into_iter()
                    .filter(|item| query.is_empty() || matches_query(item, &query))
                    .collect();
                Response::ok(request.id, serde_json::json!({"items": items}))
            }
            Err(message) => Response::err(request.id, ErrorCode::Internal, message),
        },
        "bookmark.add" | "bookmark.remove" => {
            let tab_url = requested_tab_id(&request.params)
                .map(|tab| tab_url(&tab))
                .unwrap_or_default();
            let url = request
                .params
                .get("url")
                .and_then(|value| value.as_str())
                .map(str::to_string)
                .unwrap_or(tab_url);
            if !url.starts_with("http") {
                return Response::err(
                    request.id,
                    ErrorCode::InvalidParams,
                    "only http(s) pages can be bookmarked",
                );
            }
            let title = request
                .params
                .get("title")
                .and_then(|value| value.as_str())
                .map(str::to_string)
                .or_else(|| {
                    tab_field(&active_tab_id(), |tab| tab.title.clone())
                        .filter(|title| !title.is_empty())
                })
                .unwrap_or_else(|| url.clone());
            let result = if request.method == "bookmark.add" {
                with_store(|store| store.add_bookmark(&url, &title, unix_now())).map(|_| true)
            } else {
                with_store(|store| store.remove_bookmark(&url)).map(|_| false)
            };
            match result {
                Ok(saved) => {
                    notify_shell_state();
                    Response::ok(
                        request.id,
                        serde_json::json!({"url": url, "bookmarked": saved}),
                    )
                }
                Err(message) => Response::err(request.id, ErrorCode::Internal, message),
            }
        }
        "audit.list" => match with_store(|store| {
            store.audit_events(
                request
                    .params
                    .get("limit")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(50) as usize,
            )
        }) {
            Ok(items) => Response::ok(request.id, serde_json::json!({"items": items})),
            Err(message) => Response::err(request.id, ErrorCode::Internal, message),
        },
        "agent.connection" => Response::ok(
            request.id,
            serde_json::json!({
                "pipe": super::pipe::pipe_name(&state().instance_id),
                "instanceId": state().instance_id,
                "tokenFile": state().paths.instance_dir(&state().instance_id).join("token").to_string_lossy(),
                "command": "agentgo call observe.page",
            }),
        ),
        "history.clear" => {
            if request
                .params
                .get("confirm")
                .and_then(|value| value.as_bool())
                != Some(true)
            {
                return Response::err(
                    request.id,
                    ErrorCode::ApprovalRequired,
                    "history.clear requires params.confirm=true",
                );
            }
            clear_history();
            Response::ok(request.id, serde_json::json!({"cleared":true}))
        }
        "downloads.list" | "download.list" => Response::ok(
            request.id,
            serde_json::json!({"items": downloads_snapshot()}),
        ),
        "downloads.clear" | "download.clear" => {
            if request
                .params
                .get("confirm")
                .and_then(|value| value.as_bool())
                != Some(true)
            {
                return Response::err(
                    request.id,
                    ErrorCode::ApprovalRequired,
                    "downloads.clear requires params.confirm=true",
                );
            }
            clear_downloads();
            Response::ok(request.id, serde_json::json!({"cleared":true}))
        }
        "download.start" => match start_download(&request.params) {
            Ok(result) => Response::ok(request.id, result),
            Err(message) if message.starts_with("downloads are disabled") => {
                Response::err(request.id, ErrorCode::ApprovalRequired, message)
            }
            Err(message) => Response::err(request.id, ErrorCode::InvalidParams, message),
        },
        "download.progress" => match download_progress(&request.params) {
            Ok(result) => Response::ok(request.id, result),
            Err(message) => Response::err(request.id, ErrorCode::InvalidParams, message),
        },
        "download.pause" | "download.resume" | "download.cancel" => {
            match control_download(request.method.as_str(), &request.params) {
                Ok(result) => Response::ok(request.id, result),
                Err(message) => Response::err(request.id, ErrorCode::TargetStale, message),
            }
        }
        "browser.crash_report" => Response::ok(request.id, crash_reports_snapshot()),
        "search.resolve" | "navigate.search" => {
            let input = request
                .params
                .get("input")
                .or_else(|| request.params.get("query"))
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty());
            let Some(input) = input else {
                return Response::err(
                    request.id,
                    ErrorCode::InvalidParams,
                    "search.resolve requires params.input or params.query",
                );
            };
            let url = resolve_search_input(input);
            Response::ok(
                request.id,
                serde_json::json!({"url":url,"engine":search_engine().0}),
            )
        }
        "siteinfo.get" => {
            let tab_id = match requested_tab_id(&request.params) {
                Ok(id) => id,
                Err(message) => return Response::err(request.id, ErrorCode::TargetStale, message),
            };
            let snapshot = match content_tab_snapshot_for(&tab_id) {
                Some(snapshot) => snapshot,
                None => {
                    return Response::err(
                        request.id,
                        ErrorCode::TargetStale,
                        format!("unknown tab id {tab_id}"),
                    )
                }
            };
            let url = snapshot
                .get("url")
                .and_then(|value| value.as_str())
                .unwrap_or("about:blank");
            Response::ok(request.id, site_info(&tab_id, url))
        }
        "api.capabilities" | "browser.capabilities" => {
            let registry = agentgo_shell_bridge::ShellMethodRegistry::new();
            let names = |status| {
                registry
                    .all()
                    .iter()
                    .filter(|method| method.status == status)
                    .map(|method| method.name)
                    .collect::<Vec<_>>()
            };
            Response::ok(
                request.id,
                serde_json::json!({
                    "protocol": agentgo_protocol::PROTOCOL_VERSION,
                    "implemented": names(agentgo_shell_bridge::MethodStatus::Implemented),
                    "planned": names(agentgo_shell_bridge::MethodStatus::Planned),
                    "manifest": "schemas/capability-manifest.json",
                }),
            )
        }
        "tabs.navigate" | "tab.navigate" | "navigate.url" => {
            let tab_id = match requested_tab_id(&request.params) {
                Ok(id) => id,
                Err(message) => return Response::err(request.id, ErrorCode::TargetStale, message),
            };
            let url = request
                .params
                .get("url")
                .and_then(|value| value.as_str())
                .map(str::to_string);
            let Some(url) = url else {
                return Response::err(
                    request.id,
                    ErrorCode::InvalidParams,
                    "navigation requires params.url",
                );
            };
            if !is_navigable_url(&url) {
                return Response::err(
                    request.id,
                    ErrorCode::InvalidParams,
                    "url must use http, https, file, or about",
                );
            }
            navigate_tab(&tab_id, &url);
            Response::ok(
                request.id,
                serde_json::json!({"tabId":tab_id,"url":url,"accepted":true}),
            )
        }
        "tabs.list" | "tab.list" => Response::ok(
            request.id,
            serde_json::json!({
                "activeTabId": active_tab_id(),
                "tabs": tabs_snapshot()
            }),
        ),
        "tabs.status" | "tab.status" => {
            let tab_id = match requested_tab_id(&request.params) {
                Ok(id) => id,
                Err(message) => return Response::err(request.id, ErrorCode::TargetStale, message),
            };
            match content_tab_snapshot_for(&tab_id) {
                Some(snapshot) => Response::ok(request.id, snapshot),
                None => Response::err(request.id, ErrorCode::TargetStale, "unknown tab id"),
            }
        }
        "tabs.create" | "tab.create" => {
            let url = request
                .params
                .get("url")
                .and_then(|value| value.as_str())
                .unwrap_or("about:blank");
            let placement = if opened_by_agent(&request.params) {
                agent_tab_placement()
            } else {
                Placement::Front
            };
            match create_content_tab_placed(url, placement) {
                Ok(result) => Response::ok(request.id, result),
                Err(message) => Response::err(request.id, ErrorCode::InvalidParams, message),
            }
        }
        "tabs.close" | "tab.close" => {
            let tab_id = request.params.get("tabId").and_then(|v| v.as_str());
            let Some(tab_id) = tab_id else {
                return Response::err(
                    request.id,
                    ErrorCode::InvalidParams,
                    "tabs.close requires params.tabId",
                );
            };
            match close_content_tab(tab_id) {
                Ok(result) => Response::ok(request.id, result),
                Err(message) => Response::err(request.id, ErrorCode::InvalidParams, message),
            }
        }
        "tabs.activate" | "tab.activate" => {
            let tab_id = request.params.get("tabId").and_then(|v| v.as_str());
            let Some(tab_id) = tab_id else {
                return Response::err(
                    request.id,
                    ErrorCode::InvalidParams,
                    "tabs.activate requires params.tabId",
                );
            };
            let activated = if opened_by_agent(&request.params) {
                activate_tab_for_agent(tab_id)
            } else {
                activate_tab(tab_id).map(|()| {
                    serde_json::json!({
                        "accepted": true,
                        "applied": true,
                        "activeTabId": active_tab_id()
                    })
                })
            };
            match activated {
                Ok(result) => Response::ok(request.id, result),
                Err(message) => Response::err(request.id, ErrorCode::TargetStale, message),
            }
        }
        "window.status" => match window_state() {
            Ok(result) => Response::ok(request.id, result),
            Err(message) => Response::err(request.id, ErrorCode::TargetStale, message),
        },
        "tab.reload" | "tab.stop" | "tab.back" | "tab.forward" => {
            let tab_id = match requested_tab_id(&request.params) {
                Ok(id) => id,
                Err(message) => return Response::err(request.id, ErrorCode::TargetStale, message),
            };
            let ignore_cache = request
                .params
                .get("ignoreCache")
                .and_then(|value| value.as_bool())
                .unwrap_or(false);
            let changed = match request.method.as_str() {
                "tab.reload" if ignore_cache => {
                    content_browser_action_for(&tab_id, BrowserAction::ReloadIgnoreCache)
                }
                "tab.reload" => content_browser_action_for(&tab_id, BrowserAction::Reload),
                "tab.stop" => content_browser_action_for(&tab_id, BrowserAction::Stop),
                "tab.back" => content_browser_action_for(&tab_id, BrowserAction::Back),
                "tab.forward" => content_browser_action_for(&tab_id, BrowserAction::Forward),
                _ => false,
            };
            Response::ok(
                request.id,
                serde_json::json!({
                    "tabId":tab_id,
                    "accepted":changed,
                    "status":content_tab_snapshot_for(&tab_id)
                }),
            )
        }
        "window.minimize"
        | "window.maximize"
        | "window.restore"
        | "window.fullscreen"
        | "window.always_on_top" => {
            let on = request.params.get("on").and_then(|value| value.as_bool());
            let action = match request.method.as_str() {
                "window.minimize" => WindowAction::Minimize,
                "window.maximize" => WindowAction::Maximize,
                "window.restore" => WindowAction::Restore,
                "window.fullscreen" => WindowAction::Fullscreen(on.unwrap_or_else(|| {
                    window_action_state().is_some_and(|state| state["fullscreen"] != true)
                })),
                _ => WindowAction::AlwaysOnTop(on.unwrap_or(true)),
            };
            match window_action(action) {
                Some(snapshot) => Response::ok(request.id, snapshot),
                None => Response::err(
                    request.id,
                    ErrorCode::TargetStale,
                    "browser window is not ready",
                ),
            }
        }
        "window.resize" => match window_resize(&request.params) {
            Ok(result) => Response::ok(request.id, result),
            Err(message) => Response::err(request.id, ErrorCode::InvalidParams, message),
        },
        "browser.shutdown" | "window.close" => {
            request_window_close(0);
            Response::ok(request.id, serde_json::json!({"closing":true}))
        }
        method => {
            if agentgo_shell_bridge::ShellMethodRegistry::new()
                .lookup(method)
                .is_some()
            {
                Response::err(
                    request.id,
                    ErrorCode::CapabilityUnsupported,
                    format!("{method} is not implemented in this build"),
                )
            } else {
                Response::err(
                    request.id,
                    ErrorCode::MethodNotFound,
                    format!("unknown method {method}"),
                )
            }
        }
    }
}

/// The dispatcher marks a request that came from an agent session. The
/// marker is stripped from agent input first, so an agent cannot claim to be
/// the operator by sending it.
fn opened_by_agent(params: &serde_json::Value) -> bool {
    params.get(AGENT_MARK).and_then(|value| value.as_bool()) == Some(true)
}

/// Internal request parameter set by the dispatcher for agent sessions.
pub(crate) const AGENT_MARK: &str = "_byAgent";

pub(crate) fn is_navigable_url(url: &str) -> bool {
    if ["http://", "https://", "file://", "about:"]
        .iter()
        .any(|prefix| url.starts_with(prefix))
    {
        return true;
    }
    url.split(['?', '#']).next() == Some("agentgo://shell/index.html")
}

/// Whether a history or bookmark entry's title or address contains `query`
/// (already lower case).
fn matches_query(item: &serde_json::Value, query: &str) -> bool {
    ["title", "url"].iter().any(|key| {
        item[*key]
            .as_str()
            .is_some_and(|text| text.to_lowercase().contains(query))
    })
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

/// Search providers the operator may pick in Settings. Google is the default
/// (ADR-0006); there is no AgentGo search.
pub const SEARCH_ENGINES: [(&str, &str); 3] = [
    ("Google", "https://www.google.com/search?q="),
    ("Bing", "https://www.bing.com/search?q="),
    ("DuckDuckGo", "https://duckduckgo.com/?q="),
];

/// The selected provider's name and query prefix.
pub fn search_engine() -> (&'static str, &'static str) {
    let chosen = settings_snapshot()["searchEngine"]
        .as_str()
        .unwrap_or("Google")
        .to_string();
    SEARCH_ENGINES
        .into_iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(&chosen))
        .unwrap_or(SEARCH_ENGINES[0])
}

pub(crate) fn resolve_search_input(input: &str) -> String {
    resolve_search_with(input, search_engine().1)
}

fn resolve_search_with(input: &str, base: &str) -> String {
    if is_navigable_url(input) {
        return input.to_string();
    }
    if input.contains('.') && !input.chars().any(char::is_whitespace) {
        return format!("https://{input}");
    }
    format!("{base}{}", query_encode(input))
}

fn site_info(tab_id: &str, url: &str) -> serde_json::Value {
    let (scheme, remainder) = url.split_once("://").unwrap_or((url, ""));
    let host = remainder.split(['/', '?', '#']).next().unwrap_or("");
    let granted = settings_snapshot()["sitePermissions"][host].clone();
    serde_json::json!({
        "tabId": tab_id,
        "url": url,
        "scheme": scheme,
        "host": host,
        "secure": scheme.eq_ignore_ascii_case("https"),
        "certificate": certificate_info(tab_id),
        "bridge": false,
        "permissions": if granted.is_null() { serde_json::json!([]) } else { granted }
    })
}

/// The certificate of the page a tab shows: who it was issued to and by,
/// and until when it is valid (null for pages without one).
fn certificate_info(tab_id: &str) -> serde_json::Value {
    if STATE.get().is_none() {
        return serde_json::Value::Null;
    }
    let Some(status) = content_host(tab_id)
        .and_then(|host| host.visible_navigation_entry())
        .and_then(|entry| entry.sslstatus())
    else {
        return serde_json::Value::Null;
    };
    let Some(certificate) = status.x509_certificate() else {
        return serde_json::Value::Null;
    };
    let name = |principal: Option<X509CertPrincipal>| {
        principal
            .map(|p| CefString::from(&p.common_name()).to_string())
            .unwrap_or_default()
    };
    // Microseconds since 1601-01-01 to seconds since 1970-01-01.
    let expiry = certificate.valid_expiry().val / 1_000_000 - 11_644_473_600;
    serde_json::json!({
        "secureConnection": status.is_secure_connection() != 0,
        "issuedTo": name(certificate.subject()),
        "issuedBy": name(certificate.issuer()),
        "validUntil": expiry.max(0),
    })
}

/// The toolbar's site chip was clicked: show what "Secure" (or "Not
/// secure") means for this page, under the chip at `x`.
pub fn show_site_info(x: i32) {
    let tab = active_tab_id();
    let url = tab_url(&tab);
    SUGGEST.with(|s| {
        let (_, width, rows) = *s.borrow();
        *s.borrow_mut() = (x, width, rows);
    });
    let _ = show_tool("siteinfo", "", &site_info(&tab, &url));
    relayout_tool_surface();
}

pub(crate) fn content_tab_snapshot_for(id: &str) -> Option<serde_json::Value> {
    let active = active_tab_id() == id;
    let (view, fallback_url, fallback_loading, crashed, unresponsive, last_crash, title, favicon) =
        state().content_tabs.lock().ok().and_then(|tabs| {
            tabs.iter().find(|tab| tab.id == id).map(|tab| {
                let fallback_url = if active {
                    state()
                        .content_url
                        .lock()
                        .map(|url| url.clone())
                        .unwrap_or_else(|_| "about:blank".to_string())
                } else {
                    "about:blank".to_string()
                };
                (
                    tab.view.0.clone(),
                    fallback_url,
                    if active {
                        state().content_loading.load(Ordering::SeqCst)
                    } else {
                        false
                    },
                    tab.crashed,
                    tab.unresponsive,
                    tab.last_crash.clone(),
                    tab.title.clone(),
                    tab.favicon.clone(),
                )
            })
        })?;
    let zoom = view
        .browser()
        .and_then(|browser| browser.host())
        .map(|host| (1.2_f64.powf(host.zoom_level()) * 100.0).round())
        .unwrap_or(100.0);
    let (url, loading, can_go_back, can_go_forward) = view
        .browser()
        .map(|browser| {
            let url = browser
                .main_frame()
                .map(|frame| CefString::from(&frame.url()).to_string())
                .filter(|url| !url.is_empty())
                .unwrap_or(fallback_url.clone());
            (
                url,
                browser.is_loading() != 0,
                browser.can_go_back() != 0,
                browser.can_go_forward() != 0,
            )
        })
        .unwrap_or((fallback_url, fallback_loading, false, false));
    Some(serde_json::json!({
        "tabId": id,
        "url": url,
        "title": title,
        "favicon": favicon,
        "agent": super::sessions::lease_holder(id),
        "handoff": super::sessions::handoff(id).map(|(_, reason)| reason),
        "saveLogin": super::accounts::offer_for(id),
        "hasLogin": super::accounts::has_login(&tab_url(id)),
        "injection": super::page::injection_level(id),
        "loading": loading,
        "canGoBack": can_go_back,
        "canGoForward": can_go_forward,
        "zoom": zoom,
        "active": active,
        "crashed": crashed,
        "unresponsive": unresponsive,
        "lastCrash": last_crash,
        "kind": "content",
        "strip": super::winmgr::strip_state(id),
    }))
}

pub(crate) fn tabs_snapshot() -> Vec<serde_json::Value> {
    let ids = state()
        .content_tabs
        .lock()
        .map(|tabs| tabs.iter().map(|tab| tab.id.clone()).collect::<Vec<_>>())
        .unwrap_or_default();
    ids.iter()
        .filter_map(|id| content_tab_snapshot_for(id))
        .collect()
}

#[derive(Clone, Copy)]
enum BrowserAction {
    Reload,
    ReloadIgnoreCache,
    Stop,
    Back,
    Forward,
}

/// Executes a navigation control on the CEF/UI thread and reports whether it
/// was accepted. The checks avoid turning a harmless back/forward click into
/// a misleading success when no history entry exists.
fn content_browser_action_for(tab_id: &str, action: BrowserAction) -> bool {
    let Some(view) = content_view_for_tab(tab_id) else {
        return false;
    };
    let Some(browser) = view.browser() else {
        return false;
    };
    match action {
        BrowserAction::Reload => {
            browser.reload();
            true
        }
        BrowserAction::ReloadIgnoreCache => {
            browser.reload_ignore_cache();
            true
        }
        BrowserAction::Stop => {
            if browser.is_loading() == 0 {
                false
            } else {
                browser.stop_load();
                state().content_loading.store(false, Ordering::SeqCst);
                true
            }
        }
        BrowserAction::Back => {
            if browser.can_go_back() == 0 {
                false
            } else {
                browser.go_back();
                state().content_loading.store(true, Ordering::SeqCst);
                true
            }
        }
        BrowserAction::Forward => {
            if browser.can_go_forward() == 0 {
                false
            } else {
                browser.go_forward();
                state().content_loading.store(true, Ordering::SeqCst);
                true
            }
        }
    }
}

/// Updates the navigation state for one tab before its frame load begins.
pub fn set_content_navigation_for_tab(tab_id: &str, url: &str) {
    if let Ok(mut current) = state().content_url.lock() {
        if active_tab_id() == tab_id {
            *current = url.to_string();
        }
    }
    if active_tab_id() == tab_id {
        state().content_loading.store(true, Ordering::SeqCst);
    }
    notify_shell_state();
}

/// Marks one content tab as settled after a successful main-frame load.
pub fn set_content_loaded_for_tab(tab_id: &str, url: &str) {
    if let Ok(mut current) = state().content_url.lock() {
        if active_tab_id() == tab_id {
            *current = url.to_string();
        }
    }
    if active_tab_id() == tab_id {
        state().content_loading.store(false, Ordering::SeqCst);
    }
    record_history(tab_id, url);
    notify_shell_state();
    super::sessions::emit(
        "tab.loaded",
        serde_json::json!({"tabId": tab_id, "url": url}),
    );
}

fn tab_field<T>(tab_id: &str, read: impl FnOnce(&ContentTab) -> T) -> Option<T> {
    state()
        .content_tabs
        .lock()
        .ok()
        .and_then(|tabs| tabs.iter().find(|tab| tab.id == tab_id).map(read))
}

/// Records a visit with the best title known so far. Private (ephemeral)
/// runs keep history in memory only.
fn record_history(tab_id: &str, url: &str) {
    if url.starts_with("agentgo://") || url == "about:blank" {
        return;
    }
    let title = tab_field(tab_id, |tab| tab.title.clone())
        .filter(|title| !title.is_empty())
        .unwrap_or_else(|| url.to_string());
    if let Ok(mut history) = state().history.lock() {
        history.retain(|item| item.get("url").and_then(|v| v.as_str()) != Some(url));
        history.insert(
            0,
            serde_json::json!({"url": url, "title": title, "visitedAt": unix_now()}),
        );
        history.truncate(100);
    }
    if !state().ephemeral_profile.load(Ordering::SeqCst) {
        if let Ok(store) = SqliteStateStore::open(&state().history_path) {
            let _ = store.record_visit(url, &title, unix_now());
        }
    }
}

/// Stores the renderer-reported title and refreshes history and the chrome.
pub fn set_tab_title(tab_id: &str, title: &str) {
    let title: String = title.chars().take(300).collect();
    if let Ok(mut tabs) = state().content_tabs.lock() {
        if let Some(tab) = tabs.iter_mut().find(|tab| tab.id == tab_id) {
            if tab.title == title {
                return;
            }
            tab.title = title;
        }
    }
    let url = tab_url(tab_id);
    if !tab_is_loading(tab_id) {
        record_history(tab_id, &url);
    }
    notify_shell_state();
}

/// Stores the renderer-reported favicon URL.
pub fn set_tab_favicon(tab_id: &str, favicon: Option<String>) {
    if let Ok(mut tabs) = state().content_tabs.lock() {
        if let Some(tab) = tabs.iter_mut().find(|tab| tab.id == tab_id) {
            tab.favicon = favicon;
        }
    }
    notify_shell_state();
}

/// Records a main-frame load failure for the next navigation reply.
pub fn set_load_error(tab_id: &str, error: &str) {
    if let Ok(mut tabs) = state().content_tabs.lock() {
        if let Some(tab) = tabs.iter_mut().find(|tab| tab.id == tab_id) {
            tab.load_error = Some(error.chars().take(200).collect());
        }
    }
}

/// Address of the bundled page shown in place of one that failed to load.
pub fn error_page_url(failed_url: &str, error: &str) -> String {
    format!(
        "agentgo://shell/error.html?url={}&error={}&theme={}&k={}",
        query_encode(failed_url),
        query_encode(error),
        // the error page has no bridge; it is told the person's theme
        match settings_snapshot()["theme"].as_str() {
            Some("light") => "light",
            Some("system") => "system",
            _ => "dark",
        },
        error_page_key()
    )
}

/// A per-run key on every error page AgentGo shows itself. A web page cannot
/// guess it, so it cannot open a look-alike AgentGo warning of its own.
fn error_page_key() -> &'static str {
    static KEY: OnceLock<String> = OnceLock::new();
    KEY.get_or_init(|| {
        let mut bytes = [0u8; 16];
        let _ = getrandom::fill(&mut bytes);
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    })
}

/// Whether `url` is an error page AgentGo made (it carries this run's key).
pub(crate) fn is_own_error_page(url: &str) -> bool {
    url.strip_prefix("agentgo://shell/error.html?")
        .map(|rest| rest.split('#').next().unwrap_or(""))
        .is_some_and(|query| {
            query
                .split('&')
                .any(|pair| pair.strip_prefix("k=") == Some(error_page_key()))
        })
}

fn session_file() -> PathBuf {
    state().options.cache_root.join("session.json")
}

/// The pages to reopen: what was open when the browser last closed.
fn saved_session() -> Vec<String> {
    std::fs::read(session_file())
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Vec<String>>(&bytes).ok())
        .unwrap_or_default()
        .into_iter()
        .filter(|url| is_navigable_url(url))
        .take(50)
        .collect()
}

/// Keeps the list of open pages on disk, so they can be reopened next time.
fn remember_session() {
    thread_local! {
        static LAST: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
    }
    if state().quit_requested.load(Ordering::SeqCst) || state().options.smoke {
        return;
    }
    let open: Vec<String> = content_tab_ids()
        .iter()
        .map(|id| tab_url(id))
        .map(|url| {
            // A page that failed is remembered by the address that was asked for.
            match url.strip_prefix("agentgo://shell/error.html?url=") {
                Some(rest) => query_decode(rest.split('&').next().unwrap_or("")),
                None => url,
            }
        })
        .filter(|url| url.starts_with("http"))
        .collect();
    if open.is_empty() || LAST.with(|last| *last.borrow() == open) {
        return;
    }
    if let Ok(bytes) = serde_json::to_vec(&open) {
        let _ = std::fs::write(session_file(), bytes);
    }
    LAST.with(|last| *last.borrow_mut() = open);
}

fn query_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let hex = (bytes[index] == b'%')
            .then(|| bytes.get(index + 1..index + 3))
            .flatten()
            .and_then(|pair| std::str::from_utf8(pair).ok())
            .and_then(|pair| u8::from_str_radix(pair, 16).ok());
        match hex {
            Some(byte) => {
                out.push(byte);
                index += 3;
            }
            None => {
                out.push(bytes[index]);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Takes the last load error of a tab.
pub fn take_load_error(tab_id: &str) -> Option<String> {
    state().content_tabs.lock().ok().and_then(|mut tabs| {
        tabs.iter_mut()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| tab.load_error.take())
    })
}

/// Whether the tab's browser is still loading.
pub fn tab_is_loading(tab_id: &str) -> bool {
    content_view_for_tab(tab_id)
        .and_then(|view| view.browser())
        .is_some_and(|browser| browser.is_loading() != 0)
}

/// Current main-frame URL of a tab.
pub fn tab_url(tab_id: &str) -> String {
    content_view_for_tab(tab_id)
        .and_then(|view| view.browser())
        .and_then(|browser| browser.main_frame())
        .map(|frame| CefString::from(&frame.url()).to_string())
        .unwrap_or_default()
}

/// Effective control mode: the operator's launch flag wins over the setting.
pub fn control_mode() -> String {
    state().options.control_mode.clone().unwrap_or_else(|| {
        settings_snapshot()
            .get("controlMode")
            .and_then(|value| value.as_str())
            .unwrap_or("approval")
            .to_string()
    })
}

/// Whether an operator launch grant is active.
pub fn option_enabled(name: &str) -> bool {
    match name {
        "allowEvaluate" => state().options.allow_evaluate,
        "allowDownloads" => state().options.allow_downloads,
        _ => false,
    }
}

/// Whether a local path lies inside a directory granted for uploads.
pub fn upload_allowed(path: &str) -> bool {
    let Ok(path) = std::path::Path::new(path).canonicalize() else {
        return false;
    };
    // `AGENTGO_UPLOAD_DIRS` (`;`-separated) is the same operator grant as
    // `--upload-dir`, for hosts that start the browser indirectly (an MCP
    // server, a test harness). It comes from the environment of whoever
    // launched AgentGo, never from an agent.
    let from_env = std::env::var("AGENTGO_UPLOAD_DIRS").unwrap_or_default();
    state()
        .options
        .upload_dirs
        .iter()
        .cloned()
        .chain(
            from_env
                .split(';')
                .filter(|s| !s.is_empty())
                .map(PathBuf::from),
        )
        .any(|dir| dir.canonicalize().is_ok_and(|dir| path.starts_with(dir)))
}

/// Allocates a new artifact file path for a session.
pub fn new_artifact_path(session: &str, kind: &str, extension: &str) -> std::io::Result<PathBuf> {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let safe: String = session
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect();
    let dir = state()
        .paths
        .artifact_dir(&format!("{}-{safe}", state().instance_id));
    std::fs::create_dir_all(&dir)?;
    let number = NEXT.fetch_add(1, Ordering::SeqCst);
    Ok(dir.join(format!("{kind}-{number:04}.{extension}")))
}

/// Appends an agent action to the local audit log.
pub fn audit(entry: &serde_json::Value) {
    if let Ok(store) = SqliteStateStore::open(&state().history_path) {
        let _ = store.record_audit("agent.action", &entry.to_string(), unix_now());
    }
}

/// Profile facts for `session.info` and health.
pub fn profile_info() -> serde_json::Value {
    serde_json::json!({
        "id": active_profile_id(),
        "ephemeral": state().ephemeral_profile.load(Ordering::SeqCst),
    })
}

/// Brings the window to the front on the tab that waits for the person (a
/// handoff), with the agent panel open. Called when they click AgentGo's
/// notification.
pub fn show_waiting_tab() {
    if let Some(tab) = super::sessions::waiting_tab() {
        let _ = activate_tab(&tab);
    }
    let window = state()
        .window
        .lock()
        .ok()
        .and_then(|slot| slot.as_ref().map(|handle| handle.0.clone()));
    if let Some(window) = window {
        if window.is_minimized() != 0 {
            window.restore();
        }
        window.show();
        window.activate();
    }
    present_agent_panel();
    log_event("AG-RUNTIME notification-clicked");
}

/// Tells the operator that a human is needed: flashes the taskbar button and
/// shows a Windows notification.
pub fn notify_operator(message: &str) {
    log_event(&format!("AG-RUNTIME operator-notification {message:?}"));
    if settings_snapshot()
        .get("captchaNotifications")
        .and_then(|value| value.as_bool())
        == Some(false)
    {
        return;
    }
    let window = state()
        .window
        .lock()
        .ok()
        .and_then(|slot| slot.as_ref().map(|handle| handle.0.clone()));
    if let Some(window) = window {
        super::notify::attention(&window, message);
    }
}

/// Read-only summary for the New Tab page: nothing here is secret and
/// nothing can be changed through it.
pub fn newtab_state() -> serde_json::Value {
    let status = super::sessions::status();
    let settings = settings_snapshot();
    serde_json::json!({
        "theme": settings["theme"],
        "accent": settings["accent"],
        "motion": settings["motion"],
        "searchBase": search_engine().1,
        "version": env!("CARGO_PKG_VERSION"),
        "chromium": crate::CEF_CHROMIUM_VERSION,
        "control": status["control"],
        "controlMode": control_mode(),
        // Connected agents (agent.connect or a long-lived connection), the same
        // list the toolbar pill shows, so one-shot commands never flicker it.
        "agents": status["agents"].as_array().map(|agents| agents.iter().map(|agent| serde_json::json!({
            "client": agent["agent"], "since": agent["since"],
        })).collect::<Vec<_>>()).unwrap_or_default(),
        "activity": status["activity"].as_array().map(|items| items.iter().take(6).cloned().collect::<Vec<_>>()).unwrap_or_default(),
        "recent": history_snapshot(8),
        "bookmarks": with_store(|store| store.bookmarks(12)).unwrap_or_default(),
        "profile": profile_info(),
        "pipe": super::pipe::pipe_name(&state().instance_id),
    })
}

/// Opens a page-requested popup as a normal tab leased like its opener.
pub fn open_popup_as_tab(url: &str) {
    if !is_navigable_url(url) {
        log_event(&format!("AG-RUNTIME popup-blocked {url:?}"));
        return;
    }
    let opener = active_tab_id();
    let url = url.to_string();
    // Creating a BrowserView inside the popup callback re-enters CEF; do it
    // on the next executor tick instead.
    super::exec::spawn(async move {
        match create_content_tab(&url) {
            Ok(created) => {
                if let (Some(holder), Some(tab)) = (
                    super::sessions::lease_holder(&opener),
                    created["tabId"].as_str(),
                ) {
                    let _ = super::sessions::authorize_tab(
                        &super::sessions::Caller { session: holder },
                        tab,
                    );
                }
                super::sessions::emit(
                    "tab.opened",
                    serde_json::json!({"tabId": created["tabId"], "opener": opener, "url": url}),
                );
            }
            Err(error) => log_event(&format!("AG-RUNTIME popup-tab-failed {error}")),
        }
    });
    wake_ui();
}

/// Bookmark and audit accessors used by the dispatcher.
pub fn with_store<T>(
    read: impl FnOnce(&SqliteStateStore) -> Result<T, agentgo_storage::SqliteStoreError>,
) -> Result<T, String> {
    let store = SqliteStateStore::open(&state().history_path).map_err(|error| error.to_string())?;
    read(&store).map_err(|error| error.to_string())
}

/// Advances the smoke state machine. Called from the load handlers and
/// the bridge handler (all may run on CEF threads; transitions are
/// serialized through the mutex).
pub fn advance_smoke(step: SmokeStep, detail: &str) {
    if !state().options.smoke {
        return;
    }
    let Ok(mut guard) = state().smoke_step.lock() else {
        return;
    };
    if *guard != step {
        return; // stale event from an earlier step
    }
    log_event(&format!("AG-SMOKE step-advanced {step:?} {detail}"));
    match step {
        SmokeStep::AwaitShell => {
            // Shell loaded: trigger the real content navigation to Google.
            *guard = SmokeStep::AwaitContent;
            navigate_content(&state().options.content_start_url());
        }
        SmokeStep::AwaitContent => {
            // Google loaded: drive the trusted-origin bridge command to
            // navigate the content surface to the fixture page.
            *guard = SmokeStep::AwaitBridgeResult;
            drive_bridge_navigation();
        }
        SmokeStep::AwaitBridgeResult => {
            *guard = SmokeStep::Done;
            request_window_close(0);
        }
        SmokeStep::Done => {}
    }
}

/// Navigates the content surface (UI thread).
pub fn navigate_content(url: &str) {
    navigate_tab(&active_tab_id(), url);
}

/// Navigates one registered content tab on the CEF/UI thread.
pub fn navigate_tab(tab_id: &str, url: &str) {
    state().navigation_requested.store(true, Ordering::SeqCst);
    let mut task = NavigateContentTask::new(tab_id.to_string(), url.to_string());
    if cef::post_task(cef::ThreadId::UI, Some(&mut task)) == 0 {
        log_event("AG-RUNTIME content-navigation-task-rejected");
    }
}

/// Performs a content navigation after CEF has scheduled it on its UI
/// thread. Resource handlers run on an I/O thread, so this is the only
/// place that touches a browser frame for bridge-driven navigation.
fn navigate_tab_on_ui(tab_id: &str, url: &str) {
    set_content_navigation_for_tab(tab_id, url);
    if let Some(view) = content_view_for_tab(tab_id) {
        // A navigation can arrive while the window is being resized or a
        // newly active tab is still settling. Reapply visibility and bounds
        // before loading so the result cannot land in an invisible view.
        super::app::relayout_registered_content();
        if let Some(browser) = view.browser() {
            if let Some(frame) = browser.main_frame() {
                frame.load_url(Some(&cef::CefString::from(url)));
                log_event(&format!("AG-RUNTIME content-navigate tab={tab_id} {url}"));
            }
        }
    }
}

cef::wrap_task! {
    struct NavigateContentTask {
        tab_id: String,
        url: String,
    }

    impl Task {
        fn execute(&self) {
            navigate_tab_on_ui(&self.tab_id, &self.url);
        }
    }
}

/// Executes JavaScript in the shell main frame that calls the trusted
/// bridge (`fetch` to `agentgo://shell/__bridge?...`), proving the
/// command path shell -> native -> content navigation.
fn drive_bridge_navigation() {
    // The fixture page lives in the repository tests directory; resolve it
    // relative to the UI root.
    let fixture = state()
        .options
        .ui_root
        .parent()
        .map(|p| p.join("tests/fixtures/pages/fixture.html"))
        .unwrap_or_else(|| std::path::PathBuf::from("tests/fixtures/pages/fixture.html"));
    let fixture_abs = fixture.canonicalize().unwrap_or(fixture);
    let fixture_path = fixture_abs.to_string_lossy();
    // Windows canonicalization commonly returns the verbatim `\\?\C:\...`
    // form. That prefix is valid for Win32 file APIs but not inside a file URL.
    let fixture_path = fixture_path
        .strip_prefix(r"\\?\")
        .unwrap_or(fixture_path.as_ref())
        .replace('\\', "/");
    let fixture_url = format!("file:///{}", fixture_path);
    let params = query_encode(&format!("{{\"url\":\"{fixture_url}\"}}"));
    let js = format!(
        "fetch('agentgo://shell/__bridge?method=tabs.navigate&params={params}')\
         .then(r=>r.text()).then(t=>{{window.__agentgoBridgeResult=t;}})\
         .catch(e=>{{window.__agentgoBridgeError=String(e);}});"
    );
    if let Some(view) = shell_view() {
        if let Some(browser) = view.browser() {
            if let Some(frame) = browser.main_frame() {
                frame.execute_java_script(
                    Some(&cef::CefString::from(js.as_str())),
                    Some(&cef::CefString::from("agentgo://shell/index.html")),
                    0,
                );
                log_event("AG-SMOKE bridge-fetch-dispatched");
            }
        }
    }
}

/// Encodes one query parameter without adding another URL dependency to the
/// small runtime adapter.
fn query_encode(value: &str) -> String {
    value
        .bytes()
        .flat_map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                vec![byte as char]
            }
            other => format!("%{other:02X}").chars().collect(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{query_encode, resolve_search_with, site_info, SEARCH_ENGINES};

    #[test]
    fn query_encoding_preserves_only_unreserved_bytes() {
        assert_eq!(
            query_encode("{\"url\":\"file://a b\"}"),
            "%7B%22url%22%3A%22file%3A%2F%2Fa%20b%22%7D"
        );
    }

    #[test]
    fn search_resolution_is_google_first_and_url_safe() {
        assert_eq!(
            resolve_search_with("agentgo browser", SEARCH_ENGINES[0].1),
            "https://www.google.com/search?q=agentgo%20browser"
        );
        assert_eq!(
            resolve_search_with("a b", SEARCH_ENGINES[2].1),
            "https://duckduckgo.com/?q=a%20b"
        );
        assert_eq!(
            resolve_search_with("https://example.com/", SEARCH_ENGINES[0].1),
            "https://example.com/"
        );
        assert_eq!(
            resolve_search_with("example.com", SEARCH_ENGINES[0].1),
            "https://example.com"
        );
    }

    #[test]
    fn site_info_never_grants_content_bridge_access() {
        let info = site_info("content-7", "https://example.com/path");
        assert_eq!(info["tabId"], "content-7");
        assert_eq!(info["host"], "example.com");
        assert_eq!(info["secure"], true);
        assert_eq!(info["bridge"], false);
    }
}

thread_local! {
    /// (agent commands in flight, generation, lock shown) for the input lock.
    static AGENT_LOCK: std::cell::Cell<(u32, u64, bool)> = const { std::cell::Cell::new((0, 0, false)) };
}

/// While an agent acts, the pages do not take the person's mouse and keys
/// (the agent's own input reaches them through DevTools, not through the
/// window). The toolbar, Pause/Stop and Settings stay usable.
fn show_agent_lock(on: bool) {
    // A clear layer over the page area catches the person's clicks; the
    // toolbar, Settings and other panels stay above it and keep working.
    let was = AGENT_LOCK.with(|lock| lock.get().2);
    if on && !was {
        if LOCK_LAYER.with(|layer| layer.borrow().is_none()) {
            make_lock_layer();
            // Panels go back on top of the new layer.
            let _ = restack_tool_overlay();
        }
        position_lock_layer();
    }
    LOCK_LAYER.with(|layer| {
        if let Some(overlay) = layer.borrow().as_ref() {
            overlay.set_visible(i32::from(on));
        }
    });
    let payload = serde_json::json!({"on": on, "text": "Agent is working · press Pause to take over", "lock": true});
    chrome_script(&format!(
        "window.__agentgoOverlay && window.__agentgoOverlay({payload});"
    ));
    AGENT_LOCK.with(|lock| {
        let (busy, generation, _) = lock.get();
        lock.set((busy, generation, on));
    });
}

thread_local! {
    static LOCK_LAYER: std::cell::RefCell<Option<cef::OverlayController>> = const { std::cell::RefCell::new(None) };
}

/// Whether the person's input to pages is held back right now.
pub fn agent_locked() -> bool {
    AGENT_LOCK.with(|lock| lock.get().2)
}

fn make_lock_layer() {
    let Some(window) = state()
        .window
        .lock()
        .ok()
        .and_then(|slot| slot.as_ref().map(|handle| handle.0.clone()))
    else {
        return;
    };
    let Some(panel) = cef::panel_create(None) else {
        return;
    };
    // A faint tint: the page stays readable and shows it is in the agent's hands.
    panel.set_background_color(0x1A00_0000);
    let mut as_view = View::from(&panel);
    if let Some(overlay) = window.add_overlay_view(Some(&mut as_view), DockingMode::CUSTOM, 0) {
        overlay.set_visible(0);
        LOCK_LAYER.with(|layer| *layer.borrow_mut() = Some(overlay));
    }
}

fn position_lock_layer() {
    let Some(window) = state()
        .window
        .lock()
        .ok()
        .and_then(|slot| slot.as_ref().map(|handle| handle.0.clone()))
    else {
        return;
    };
    let top = shell_view().map(|view| view.bounds().height).unwrap_or(104);
    let bounds = window.bounds();
    LOCK_LAYER.with(|layer| {
        if let Some(overlay) = layer.borrow().as_ref() {
            overlay.set_bounds(Some(&cef::Rect {
                x: 0,
                y: top,
                width: bounds.width,
                height: (bounds.height - top).max(0),
            }));
        }
    });
}

/// An agent command that acts on pages starts.
pub fn agent_lock_begin() {
    AGENT_LOCK.with(|lock| {
        let (busy, generation, _) = lock.get();
        lock.set((busy + 1, generation + 1, true));
    });
    show_agent_lock(true);
}

/// It finished: the pages open again shortly after the agent's last action.
pub fn agent_lock_end() {
    let generation = AGENT_LOCK.with(|lock| {
        let (busy, generation, shown) = lock.get();
        lock.set((busy.saturating_sub(1), generation, shown));
        generation
    });
    super::exec::spawn(async move {
        super::exec::sleep(2_000).await;
        let (busy, now, shown) = AGENT_LOCK.with(std::cell::Cell::get);
        if busy == 0 && now == generation && shown {
            show_agent_lock(false);
        }
    });
}

/// The person pressed Pause or Stop: give the pages back at once.
pub fn agent_lock_release() {
    AGENT_LOCK.with(|lock| {
        let (busy, generation, _) = lock.get();
        lock.set((busy, generation + 1, false));
    });
    show_agent_lock(false);
}
