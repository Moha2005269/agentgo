//! AgentGo's first real CEF composition: a trusted local chrome surface and
//! a separately-contextualised untrusted content surface in one native
//! window. It is deliberately a spike, not a claim that all browser
//! capabilities are already complete.

use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use cef::*;

use super::scheme::ShellSchemeFactoryImpl;
use super::state::{self, RunOptions, SmokeStep, UiHandle};

/// Height reserved for the trusted AgentGo chrome surface.
/// Change this when the bundled shell response contract changes. The query
/// prevents a stale custom-scheme document cached by an earlier development
/// binary from masking a corrected MIME response.
const SHELL_START_URL: &str = "agentgo://shell/chrome.html?v=20261006-1";
/// Browser-level utilities render in their own trusted, tightly-bounded
/// BrowserView. Keeping this surface separate from the fixed chrome avoids a
/// full-window transparent overlay, which Chromium cannot composite reliably
/// over another BrowserView on Windows.
const TOOLS_START_URL: &str = "agentgo://shell/tools.html?v=20261006-1";
/// A useful desktop first-run window size. CEF converts it for the active
/// monitor DPI, and the resize callback keeps both child views in sync.
const INITIAL_WIDTH: i32 = 1360;
const INITIAL_HEIGHT: i32 = 900;

/// Dispatch a CEF subprocess before the host CLI parses Chromium's `--type`
/// switch. Returns its exit code when this is a renderer/GPU/utility process.
///
/// CEF starts these processes by re-invoking the main executable, so routing
/// them through clap would reject Chromium arguments before CEF can handle
/// them.
pub fn try_execute_subprocess() -> Option<i32> {
    try_execute_subprocess_with_sandbox(std::ptr::null_mut())
}

/// Dispatches a CEF subprocess with the sandbox object supplied by
/// `bootstrap.exe`. The direct development executable calls this with null;
/// the packaged sandbox launcher calls it with CEF's opaque sandbox pointer.
pub fn try_execute_subprocess_with_sandbox(sandbox_info: *mut u8) -> Option<i32> {
    let args = args::Args::new();
    let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);
    // Custom schemes must be declared identically in every CEF process. The
    // browser process registers the resource factory; renderer/utility
    // processes still need the App callback so they recognize `agentgo://`.
    let mut app = AgentGoApp::new(PathBuf::new(), PathBuf::new());
    let code = execute_process(Some(args.as_main_args()), Some(&mut app), sandbox_info);
    (code >= 0).then_some(code)
}

/// Entry point for the browser process. This function does not return:
/// CEF child processes exit through `execute_process` and the browser
/// process exits only after its message loop has shut down cleanly.
#[cfg(all(target_os = "windows", feature = "sandbox"))]
pub fn run_browser(_options: RunOptions) -> ! {
    // CEF's Windows sandbox contract requires bootstrap.exe/bootstrapc.exe
    // to inject sandbox_info into every process. A direct Rust exe cannot
    // provide that pointer, so fail closed instead of starting a broken
    // half-sandboxed browser that may hang or deny every navigation.
    eprintln!(
        "AgentGo sandbox build requires the packaged CEF bootstrap launcher; \
         use the regular runtime build for direct development launches"
    );
    std::process::exit(3);
}

#[cfg(not(all(target_os = "windows", feature = "sandbox")))]
pub fn run_browser(options: RunOptions) -> ! {
    run_browser_inner(options, std::ptr::null_mut())
}

/// Browser entry used by the official Windows CEF bootstrap DLL contract.
/// `bootstrap.exe` owns creation and lifetime of `sandbox_info`; AgentGo only
/// forwards the opaque pointer to CEF and never inspects it.
#[cfg(feature = "sandbox")]
pub fn run_browser_sandboxed(options: RunOptions, sandbox_info: *mut u8) -> ! {
    run_browser_inner(options, sandbox_info)
}

fn run_browser_inner(mut options: RunOptions, sandbox_info: *mut u8) -> ! {
    let args = args::Args::new();

    // CEF uses this call to select the versioned C ABI before it unwraps any
    // Rust-provided ref-counted callbacks (including `CefApp`). Leaving the
    // API version unconfigured makes CEF report version `-1` and abort with a
    // misleading breakpoint dialog on startup.
    let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);

    // CEF child processes must be dispatched before browser-process state is
    // initialized. In a packaged sandbox build CEF's bootstrap launcher calls
    // this same DLL entry with the appropriate process type. Keep the app
    // callbacks present here so every child registers the agentgo:// scheme;
    // otherwise the browser process can serve chrome.html while the renderer
    // silently fails to finish loading the trusted shell.
    let mut subprocess_app = AgentGoApp::new(options.ui_root.clone(), options.cache_root.clone());
    let child_code = execute_process(
        Some(args.as_main_args()),
        Some(&mut subprocess_app),
        sandbox_info,
    );
    if child_code >= 0 {
        std::process::exit(child_code);
    }

    state::prepare_profile_root(&mut options);
    // Settings -> Hardware acceleration: off selects the software compositor
    // for this and every child process (read by the command-line hook).
    if std::env::var_os("AGENTGO_RENDER_MODE").is_none()
        && state::software_rendering_requested(&options)
    {
        std::env::set_var("AGENTGO_RENDER_MODE", "software");
    }
    // Read by the command-line hook in every CEF process (children inherit
    // the environment): pages of an agent-started window must stay live.
    if options.background() {
        std::env::set_var("AGENTGO_PAGES_STAY_LIVE", "1");
    }

    // Chrome's ProcessSingleton owns a lock under the root cache directory.
    // Supplying an explicit, writable per-app root before `initialize` avoids
    // falling back to the executable directory (which may be read-only or
    // shared by another development run).
    if let Err(error) = std::fs::create_dir_all(&options.cache_root) {
        eprintln!(
            "AgentGo could not create its CEF cache directory {}: {error}",
            options.cache_root.display()
        );
        std::process::exit(6);
    }

    state::init_state(options.clone());
    super::notify::register_identity();
    state::log_event("AG-RUNTIME browser-process-started");

    // A direct development executable cannot claim Windows sandbox support:
    // CEF requires its bootstrap packaging path for that. The `sandbox`
    // feature is reserved for that packaged build; ordinary `runtime` builds
    // run explicitly without the OS sandbox while retaining CEF's separate
    // renderer process model.
    // Normal desktop launches use CEF's own message loop. CEF documents this
    // as the best balance of rendering performance and CPU usage. Stdio and
    // bounded smoke runs retain the external loop because they must drain the
    // local protocol channel on this application thread.
    let use_external_pump = options.stdio || options.smoke;
    // The DevTools port gives full control of every page, so release builds
    // ignore the variable unless a developer also sets AGENTGO_UNSAFE_DEVTOOLS=1.
    let remote_debugging_port = devtools_port();
    let languages = state::startup_languages(&options).unwrap_or_default();
    // Chromium's own messages (GPU driver notes, Chrome's extension registry
    // and the like) go to a file beside AgentGo's log instead of the console.
    let chromium_log = options.log_path.with_file_name("agentgo-chromium.log");
    let settings = Settings {
        log_file: CefString::from(chromium_log.to_string_lossy().as_ref()),
        log_severity: LogSeverity::WARNING,
        accept_language_list: CefString::from(languages.as_str()),
        no_sandbox: (!cfg!(feature = "sandbox")) as i32,
        external_message_pump: use_external_pump as i32,
        remote_debugging_port,
        root_cache_path: CefString::from(options.cache_root.to_string_lossy().as_ref()),
        cache_path: CefString::from(options.cache_root.to_string_lossy().as_ref()),
        persist_session_cookies: 1,
        ..Default::default()
    };

    let mut app = AgentGoApp::new(options.ui_root, options.cache_root);
    match initialize(
        Some(args.as_main_args()),
        Some(&settings),
        Some(&mut app),
        sandbox_info,
    ) {
        1 => state::log_event("AG-RUNTIME cef-initialized"),
        code => {
            state::log_event(&format!("AG-RUNTIME initialize-failed code={code}"));
            std::process::exit(code);
        }
    }

    if !use_external_pump {
        schedule_tick(0);
        // The Views window and its browsers close through the regular CEF
        // lifecycle callbacks, which call `quit_message_loop` once the final
        // native window is destroyed.
        run_message_loop();
        finish_and_exit();
    }

    // CEF asks the host for work through the external-pump callback. The
    // callback only wakes this loop; all CEF work remains on this application
    // thread. A bounded fallback keeps protocol input and shutdown responsive
    // even if a platform callback is lost.
    let smoke_deadline = options
        .smoke
        .then(|| std::time::Instant::now() + Duration::from_secs(30));
    let mut smoke_timeout_requested = false;
    watch_ui_loop();
    fine_timer();
    while !state::state()
        .quit_requested
        .load(std::sync::atomic::Ordering::SeqCst)
    {
        PHASE.store(1, std::sync::atomic::Ordering::Relaxed);
        if pump_native_messages() {
            state::request_quit(0);
            break;
        }
        PHASE.store(2, std::sync::atomic::Ordering::Relaxed);
        state::drain_protocol_requests();
        let pump_requested = state::take_message_pump_request();
        PHASE.store(3, std::sync::atomic::Ordering::Relaxed);
        do_message_loop_work();
        PHASE.store(4, std::sync::atomic::Ordering::Relaxed);
        BEATS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if pump_native_messages() {
            state::request_quit(0);
            break;
        }
        // Never block the CEF UI thread on a condition variable. The bounded
        // cadence keeps Windows input/rendering alive even when a platform
        // schedule callback is delayed or lost.
        // Keep the fallback short enough that pointer/keyboard input does not
        // wait behind a missed CEF schedule callback. Four milliseconds is a
        // bounded 250 Hz check, while still yielding the UI thread instead of
        // spinning at full CPU.
        std::thread::sleep(Duration::from_millis(if pump_requested { 1 } else { 4 }));
        if !smoke_timeout_requested
            && smoke_deadline.is_some_and(|deadline| std::time::Instant::now() >= deadline)
        {
            smoke_timeout_requested = true;
            state::log_event("AG-SMOKE timeout");
            state::request_window_close(11);
        }
    }

    finish_and_exit();
}

/// Windows sleeps in ~15.6 ms steps by default, which capped every native
/// animation near 30 fps. A 1 ms timer resolution lets the 4 ms loop cadence
/// actually hold, so frames land evenly.
#[cfg(windows)]
fn fine_timer() {
    #[link(name = "winmm")]
    extern "system" {
        fn timeBeginPeriod(period: u32) -> u32;
    }
    // SAFETY: plain Win32 call with no pointers; undone at process exit.
    unsafe {
        timeBeginPeriod(1);
    }
}

#[cfg(not(windows))]
fn fine_timer() {}

static PHASE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static BEATS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Reports a stalled UI loop (phase 1/4 = Windows messages, 2 = commands,
/// 3 = Chromium work) so a hang names where it is instead of looking like a
/// slow page.
/// UI loop heartbeats so far and the phase it is in (`browser.message_loop_status`).
fn devtools_port() -> i32 {
    if !(cfg!(debug_assertions)
        || std::env::var_os("AGENTGO_UNSAFE_DEVTOOLS").is_some_and(|v| v == "1"))
    {
        return 0;
    }
    std::env::var("AGENTGO_REMOTE_DEBUGGING_PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .filter(|port| *port >= 1024)
        .map(i32::from)
        .unwrap_or(0)
}

pub(crate) fn devtools_port_open() -> bool {
    devtools_port() != 0
}

pub(crate) fn loop_status() -> (u64, u64) {
    (
        BEATS.load(std::sync::atomic::Ordering::Relaxed),
        u64::from(PHASE.load(std::sync::atomic::Ordering::Relaxed)),
    )
}

fn watch_ui_loop() {
    std::thread::spawn(|| {
        let mut last = 0;
        let mut reported = false;
        loop {
            std::thread::sleep(Duration::from_secs(2));
            let beats = BEATS.load(std::sync::atomic::Ordering::Relaxed);
            if beats != last {
                (last, reported) = (beats, false);
            } else if !reported {
                reported = true;
                let phase = PHASE.load(std::sync::atomic::Ordering::Relaxed);
                eprintln!("AG-RUNTIME ui-thread-stalled phase={phase}");
            }
        }
    });
}

/// Stops CEF and exits. Each step is logged with its timestamp so a slow
/// shutdown shows up in the evidence log instead of looking like a hang.
fn finish_and_exit() -> ! {
    state::mark_instance_stopped();
    state::log_event("AG-RUNTIME shutdown-begin");
    shutdown();
    state::log_event("AG-RUNTIME cef-shutdown-done");
    state::remove_ephemeral_root();
    state::log_event("AG-RUNTIME ephemeral-root-removed");
    let code = state::state()
        .exit_code
        .load(std::sync::atomic::Ordering::SeqCst);
    std::process::exit(code);
}

wrap_task! {
    struct TickTask;

    impl Task {
        fn execute(&self) {
            // Busy while commands are in flight, relaxed when idle.
            let pending = state::drain_protocol_requests();
            schedule_tick(if pending > 0 { 4 } else { 40 });
        }
    }
}

fn schedule_tick(delay_ms: i64) {
    let mut task = TickTask::new();
    post_delayed_task(ThreadId::UI, Some(&mut task), delay_ms);
}

wrap_task! {
    struct KickTask;

    impl Task {
        fn execute(&self) {
            state::drain_protocol_requests();
        }
    }
}

/// Runs the command executor on the UI thread as soon as possible. Safe to
/// call from any thread once CEF is initialized.
pub(crate) fn kick_ui() {
    let mut task = KickTask::new();
    post_task(ThreadId::UI, Some(&mut task));
}

/// Dispatches pending host-window messages without blocking the CEF UI
/// thread. CEF's `do_message_loop_work` drives Chromium tasks, but a Views
/// composition window also needs the Win32 queue dispatched by its host.
/// Returning true means the operating system posted WM_QUIT.
fn pump_native_messages() -> bool {
    #[cfg(target_os = "windows")]
    {
        use std::ptr::null_mut;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE, WM_QUIT,
        };

        let mut quit = false;
        loop {
            let mut message = MSG {
                hwnd: null_mut(),
                message: 0,
                wParam: 0,
                lParam: 0,
                time: 0,
                pt: windows_sys::Win32::Foundation::POINT { x: 0, y: 0 },
            };
            let has_message = unsafe { PeekMessageW(&mut message, null_mut(), 0, 0, PM_REMOVE) };
            if has_message == 0 {
                break;
            }
            if message.message == WM_QUIT {
                quit = true;
                continue;
            }
            unsafe {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        quit
    }

    #[cfg(not(target_os = "windows"))]
    {
        false
    }
}

wrap_app! {
    struct AgentGoApp {
        ui_root: PathBuf,
        cache_root: PathBuf,
    }

    impl App {
        fn on_before_command_line_processing(
            &self,
            _process_type: Option<&CefString>,
            command_line: Option<&mut CommandLine>,
        ) {
            #[cfg(target_os = "windows")]
            if let Some(command_line) = command_line {
                // Chromium writes its log to the file set in Settings only, not
                // to the console (`--enable-logging=stderr` overrides it).
                if command_line.has_switch(Some(&CefString::from("enable-logging"))) == 0 {
                    command_line.append_switch(Some(&CefString::from("enable-logging")));
                }
                // AgentGo runs extensions itself (extensions.rs). Chromium's own
                // extension system would read Google Chrome's external
                // extension list from the registry and complain about it.
                command_line.append_switch(Some(&CefString::from("disable-extensions")));
                // Quieter and lighter: no background component updates, field
                // trial pings or reliability reports, and no idle spare
                // renderer kept warm for the next navigation. These cost
                // memory and network and give a browser shell nothing.
                for switch in [
                    "disable-background-networking",
                    "disable-component-update",
                    "disable-domain-reliability",
                    "disable-sync",
                    "no-pings",
                ] {
                    command_line.append_switch(Some(&CefString::from(switch)));
                }
                merge_switch_list(
                    command_line,
                    "disable-features",
                    "SpareRendererForSitePerProcess,OptimizationHints,OptimizationGuideModelDownloading,MediaRouter",
                );
                // The executable embeds a modern Windows compatibility
                // manifest, which allows Chromium to detect and use the real
                // GPU correctly. Keep explicit software modes as diagnostics
                // escape hatches; neither is appropriate as the default for
                // an interactive browser.
                match std::env::var("AGENTGO_RENDER_MODE").as_deref() {
                    Ok("software") => {
                        command_line.append_switch(Some(&CefString::from("disable-gpu")));
                        command_line
                            .append_switch(Some(&CefString::from("disable-gpu-compositing")));
                    }
                    Ok("swiftshader") => {
                        command_line.append_switch_with_value(
                            Some(&CefString::from("use-gl")),
                            Some(&CefString::from("angle")),
                        );
                        command_line.append_switch_with_value(
                            Some(&CefString::from("use-angle")),
                            Some(&CefString::from("swiftshader")),
                        );
                    }
                    _ => {} // "hardware" and anything else: Chromium's default
                }
                // The person's feature flags (browser.feature_flags), next start.
                if let Some(flags) = state::saved_feature_flags() {
                    for (key, switch) in [("enable", "enable-features"), ("disable", "disable-features")] {
                        let list: Vec<&str> = flags[key].as_array().into_iter().flatten().filter_map(|f| f.as_str()).collect();
                        if !list.is_empty() {
                            merge_switch_list(command_line, switch, &list.join(","));
                        }
                    }
                }
                if std::env::var_os("AGENTGO_PAGES_STAY_LIVE").is_some() {
                    // An agent-started window sits behind the person's own
                    // windows. Left alone, Chromium treats a covered window
                    // as hidden: it stops painting (screenshots stall) and
                    // slows page timers to a crawl. The agent's pages must
                    // keep running, so these protections are turned off.
                    merge_switch_list(
                        command_line,
                        "disable-features",
                        "CalculateNativeWinOcclusion",
                    );
                    for switch in [
                        "disable-backgrounding-occluded-windows",
                        "disable-renderer-backgrounding",
                        "disable-background-timer-throttling",
                    ] {
                        command_line.append_switch(Some(&CefString::from(switch)));
                    }
                }
            }
        }

        fn on_register_custom_schemes(&self, registrar: Option<&mut SchemeRegistrar>) {
            let Some(registrar) = registrar else {
                return;
            };
            let options = SchemeOptions::STANDARD.get_raw()
                | SchemeOptions::SECURE.get_raw()
                | SchemeOptions::FETCH_ENABLED.get_raw();
            let registered = registrar.add_custom_scheme(
                Some(&CefString::from("agentgo")),
                options,
            );
            if registered == 0 {
                state::log_event("AG-RUNTIME scheme-registration-failed");
            }
        }

        fn browser_process_handler(&self) -> Option<BrowserProcessHandler> {
            Some(AgentGoBrowserProcessHandler::new(
                self.ui_root.clone(),
                self.cache_root.clone(),
            ))
        }
    }
}

wrap_browser_process_handler! {
    struct AgentGoBrowserProcessHandler {
        ui_root: PathBuf,
        cache_root: PathBuf,
    }

    impl BrowserProcessHandler {
        fn on_context_initialized(&self) {
            if currently_on(ThreadId::UI) == 0 {
                state::log_event("AG-RUNTIME context-initialized-off-ui-thread");
                state::request_quit(4);
                return;
            }

            let mut factory = ShellSchemeFactoryImpl::new(Arc::new(self.ui_root.clone()));
            let registered = register_scheme_handler_factory(
                Some(&CefString::from("agentgo")),
                Some(&CefString::from("shell")),
                Some(&mut factory),
            );
            if registered == 0 {
                state::log_event("AG-RUNTIME scheme-factory-registration-failed");
                state::request_quit(5);
                return;
            }
            state::log_event("AG-RUNTIME scheme-factory-registered");

            if let Err(error) = std::fs::create_dir_all(&self.cache_root) {
                state::log_event(&format!("AG-RUNTIME cache-directory-failed {error}"));
                state::request_quit(6);
                return;
            }
            // A distinct per-profile context gives untrusted content its own
            // renderer/storage boundary while preserving cookies and site
            // data across launches. The directory is local and profile-scoped;
            // credentials are still governed by CEF/Windows credential APIs,
            // never by the generic settings store.
            // CEF requires a request-context cache path to be below the
            // process root. The root is intentionally unique per launch, so
            // parallel agent executables cannot collide. The durable profile
            // catalog remains in the canonical local AgentGo data root; a
            // future profile-session layer will add controlled cookie/state
            // migration without violating CEF's root constraint.
            // CEF only accepts a request-context cache path that is a direct
            // child of the root cache path.
            let profile_dir = self.cache_root.join("content");
            let _ = std::fs::create_dir_all(&profile_dir);
            let content_context_settings = RequestContextSettings {
                cache_path: CefString::from(profile_dir.to_string_lossy().as_ref()),
                persist_session_cookies: 1,
                ..Default::default()
            };
            // A disk-backed request context finishes initializing
            // asynchronously; the window is composed from its handler.
            if !compose_window() {
                return;
            }
            let mut handler = ContentContextHandler::new(self.ui_root.clone());
            if request_context_create_context(Some(&content_context_settings), Some(&mut handler)).is_none() {
                state::log_event("AG-RUNTIME content-context-create-failed");
                state::request_quit(7);
            }
        }

        fn on_schedule_message_pump_work(&self, delay_ms: i64) {
            state::schedule_message_pump_work(delay_ms);
        }
    }
}

wrap_request_context_handler! {
    struct ContentContextHandler {
        ui_root: PathBuf,
    }

    impl RequestContextHandler {
        fn on_request_context_initialized(&self, request_context: Option<&mut RequestContext>) {
            if let Some(context) = request_context {
                attach_first_tab(&self.ui_root, context.clone());
            }
        }
    }
}

wrap_request_context_handler! {
    struct SideContextHandler {
        done: super::exec::Slot<RequestContext>,
    }

    impl RequestContextHandler {
        fn on_request_context_initialized(&self, request_context: Option<&mut RequestContext>) {
            if let Some(context) = request_context {
                let mut factory = ShellSchemeFactoryImpl::new(Arc::new(state::state().options.ui_root.clone()));
                context.register_scheme_handler_factory(
                    Some(&CefString::from("agentgo")),
                    Some(&CefString::from("shell")),
                    Some(&mut factory),
                );
                self.done.fill(context.clone());
            }
        }
    }
}

/// Creates another request context (a side profile): on disk below the
/// root cache path, or in memory when `cache` is `None`. `done` receives it
/// once Chromium has initialized it.
pub(crate) fn create_side_context(
    cache: Option<PathBuf>,
    done: super::exec::Slot<RequestContext>,
) -> bool {
    let settings = RequestContextSettings {
        cache_path: CefString::from(
            cache
                .as_ref()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default()
                .as_str(),
        ),
        persist_session_cookies: i32::from(cache.is_some()),
        ..Default::default()
    };
    let mut handler = SideContextHandler::new(done);
    request_context_create_context(Some(&settings), Some(&mut handler)).is_some()
}

/// Attaches the first content tab once the isolated, disk-backed content
/// context is ready.
fn attach_first_tab(ui_root: &std::path::Path, content_context: RequestContext) {
    // Scheme factories are request-context scoped in CEF. The global
    // registration serves the default shell context; the content context
    // needs its own read-only asset factory so the New Tab page can load
    // there without gaining bridge privileges.
    let mut content_factory = ShellSchemeFactoryImpl::new(Arc::new(ui_root.to_path_buf()));
    let content_factory_registered = content_context.register_scheme_handler_factory(
        Some(&CefString::from("agentgo")),
        Some(&CefString::from("shell")),
        Some(&mut content_factory),
    );
    if content_factory_registered == 0 {
        state::log_event("AG-RUNTIME content-scheme-factory-registration-failed");
        state::request_quit(7);
        return;
    }
    state::log_event("AG-RUNTIME isolated-content-context-created");
    // A proxy left behind by the removed VPN is put back to the system setting.
    super::profiles::route_context(&content_context, "main", None);
    state::set_content_context(content_context);
    if let Err(error) = state::create_first_tab() {
        state::log_event(&format!(
            "AG-RUNTIME content-browser-view-create-failed {error}"
        ));
        state::request_quit(8);
        return;
    }
    state::log_event("AG-RUNTIME content-browser-view-created");
    state::start_transports();
}

/// Builds the trusted chrome, the tools overlay and the top-level window.
/// This must happen synchronously during context initialization: CEF exits
/// when startup finishes without any browser.
fn compose_window() -> bool {
    super::guard::install();
    let shell_settings = BrowserSettings::default();
    let mut shell_client = SurfaceClient::new(Surface::Shell, "shell".to_string());
    let mut shell_delegate = AgentGoBrowserViewDelegate::new("shell".to_string());
    let shell_view = match browser_view_create(
        Some(&mut shell_client),
        Some(&CefString::from(SHELL_START_URL)),
        Some(&shell_settings),
        None,
        None,
        Some(&mut shell_delegate),
    ) {
        Some(view) => view,
        None => {
            state::log_event("AG-RUNTIME shell-browser-view-create-failed");
            state::request_quit(8);
            return false;
        }
    };
    state::log_event("AG-RUNTIME shell-browser-view-created");

    let tools_settings = BrowserSettings {
        // Transparent: panels draw their own rounded, animated surface over the page.
        background_color: 0x0000_0000,
        ..Default::default()
    };
    let mut tools_client = SurfaceClient::new(Surface::Tool, "tools".to_string());
    let mut tools_delegate = AgentGoBrowserViewDelegate::new("tools".to_string());
    let tools_view = match browser_view_create(
        Some(&mut tools_client),
        Some(&CefString::from(TOOLS_START_URL)),
        Some(&tools_settings),
        None,
        None,
        Some(&mut tools_delegate),
    ) {
        Some(view) => view,
        None => {
            state::log_event("AG-RUNTIME tools-browser-view-create-failed");
            state::request_quit(8);
            return false;
        }
    };
    tools_view.set_visible(0);
    state::log_event("AG-RUNTIME tools-browser-view-created");
    if let Ok(mut slot) = state::state().shell_view.lock() {
        *slot = Some(UiHandle(shell_view.clone()));
    }
    state::set_tool_view(tools_view.clone());

    let mut window_delegate = AgentGoWindowDelegate::new(
        RefCell::new(Some(shell_view)),
        RefCell::new(Some(tools_view)),
    );
    if window_create_top_level(Some(&mut window_delegate)).is_none() {
        state::log_event("AG-RUNTIME window-create-failed");
        state::request_quit(9);
        return false;
    }
    state::log_event("AG-RUNTIME composition-window-created");
    true
}

/// Creates a content BrowserView for a newly allocated protocol tab. The
/// caller is the CEF/UI thread and must attach the returned view to the
/// composition window before exposing it to an agent.
pub(crate) fn create_content_browser_view(
    tab_id: &str,
    url: &str,
    context: &mut RequestContext,
) -> Option<BrowserView> {
    let mut client = SurfaceClient::new(Surface::Content, tab_id.to_string());
    let mut delegate = AgentGoBrowserViewDelegate::new(tab_id.to_string());
    let settings = BrowserSettings::default();
    browser_view_create(
        Some(&mut client),
        Some(&CefString::from(url)),
        Some(&settings),
        None,
        Some(context),
        Some(&mut delegate),
    )
}

/// Re-applies native flex layout and visibility after tab creation or
/// activation. The shell and page are non-overlapping Window children.
pub(crate) fn relayout_registered_content() {
    let Some(window) = state::state()
        .window
        .lock()
        .ok()
        .and_then(|slot| slot.as_ref().map(|handle| handle.0.clone()))
    else {
        return;
    };
    let Some(shell) = state::shell_view() else {
        return;
    };
    let (views, tab_ids) = state::main_window_tabs();
    layout_composition(&window, &views, &tab_ids, &shell);
}

#[derive(Clone, Copy)]
enum Surface {
    Shell,
    Tool,
    Content,
    /// The developer tools, docked into the main window.
    DevTools,
}

/// The client for the docked developer tools: it only themes the tools.
pub(crate) fn devtools_client() -> Client {
    SurfaceClient::new(Surface::DevTools, "devtools".to_string())
}

wrap_client! {
    struct SurfaceClient {
        surface: Surface,
        tab_id: String,
    }

    impl Client {
        fn drag_handler(&self) -> Option<DragHandler> {
            // Only the trusted shell can describe draggable regions for the
            // frameless top-level window. Untrusted content must never gain
            // access to native window movement.
            matches!(self.surface, Surface::Shell).then(AgentGoDragHandler::new)
        }

        fn keyboard_handler(&self) -> Option<KeyboardHandler> {
            // The chrome handles its own keys in JavaScript; pages and the
            // tools drawer need the browser shortcuts forwarded to it.
            (!matches!(self.surface, Surface::Shell | Surface::DevTools))
                .then(|| AgentGoKeyboardHandler::new(matches!(self.surface, Surface::Content)))
        }

        fn load_handler(&self) -> Option<LoadHandler> {
            Some(SurfaceLoadHandler::new(self.surface, self.tab_id.clone()))
        }

        fn display_handler(&self) -> Option<DisplayHandler> {
            matches!(self.surface, Surface::Content)
                .then(|| AgentGoDisplayHandler::new(self.tab_id.clone()))
        }

        fn life_span_handler(&self) -> Option<LifeSpanHandler> {
            matches!(self.surface, Surface::Content).then(AgentGoLifeSpanHandler::new)
        }

        fn dialog_handler(&self) -> Option<DialogHandler> {
            matches!(self.surface, Surface::Content)
                .then(|| AgentGoDialogHandler::new(self.tab_id.clone()))
        }

        fn download_handler(&self) -> Option<DownloadHandler> {
            matches!(self.surface, Surface::Content).then(AgentGoDownloadHandler::new)
        }

        fn permission_handler(&self) -> Option<PermissionHandler> {
            matches!(self.surface, Surface::Content)
                .then(|| AgentGoPermissionHandler::new(self.tab_id.clone()))
        }

        fn request_handler(&self) -> Option<RequestHandler> {
            matches!(self.surface, Surface::Content)
                .then(|| AgentGoRequestHandler::new(self.tab_id.clone()))
        }

        fn find_handler(&self) -> Option<FindHandler> {
            matches!(self.surface, Surface::Content).then(AgentGoFindHandler::new)
        }

        fn context_menu_handler(&self) -> Option<ContextMenuHandler> {
            matches!(self.surface, Surface::Content)
                .then(|| AgentGoContextMenuHandler::new(self.tab_id.clone()))
        }
    }
}

// The right-click menu is drawn by the browser in its own theme (the tools
// overlay), never by the system, and never inside the page.
cef::wrap_context_menu_handler! {
    struct AgentGoContextMenuHandler {
        tab_id: String,
    }

    impl ContextMenuHandler {
        fn run_context_menu(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            params: Option<&mut ContextMenuParams>,
            _model: Option<&mut MenuModel>,
            callback: Option<&mut RunContextMenuCallback>,
        ) -> ::std::os::raw::c_int {
            let Some(params) = params else { return 0 };
            let text = |value: CefStringUserfree| CefString::from(&value).to_string();
            let data = serde_json::json!({
                "tabId": self.tab_id,
                "x": params.xcoord(),
                "y": params.ycoord(),
                "link": text(params.unfiltered_link_url()),
                "selection": text(params.selection_text()),
                "editable": params.is_editable() != 0,
                "image": params.has_image_contents() != 0,
                "source": text(params.source_url()),
                "page": text(params.page_url()),
            });
            if let Some(callback) = callback {
                callback.cancel();
            }
            state::show_context_menu(data);
            1
        }
    }
}

// Match counts of the find bar (Chromium's own find in page).
cef::wrap_find_handler! {
    struct AgentGoFindHandler;

    impl FindHandler {
        fn on_find_result(
            &self,
            _browser: Option<&mut Browser>,
            _identifier: i32,
            count: i32,
            _selection_rect: Option<&Rect>,
            active_match_ordinal: i32,
            _final_update: i32,
        ) {
            state::find_result(count, active_match_ordinal);
        }
    }
}

// Transfers the shell document's `-webkit-app-region` map to the native CEF
// Views window. This keeps the custom AgentGo chrome movable after the native
// title bar is removed, while `no-drag` controls remain fully clickable.
cef::wrap_drag_handler! {
    struct AgentGoDragHandler;

    impl DragHandler {
        fn on_draggable_regions_changed(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            regions: Option<&[DraggableRegion]>,
        ) {
            state::set_shell_draggable_regions(regions);
        }
    }
}

/// Names a browser shortcut from a key press (the names are the chrome's
/// shortcut table in `chrome.js`) and says whether pages may never see or
/// override it, as with Chrome's Ctrl+T, Ctrl+W and Ctrl+Tab.
fn browser_shortcut(event: &KeyEvent) -> Option<(String, bool)> {
    const SHIFT: u32 = 1 << 1;
    const CTRL: u32 = 1 << 2;
    const ALT: u32 = 1 << 3;
    const CTRL_SHIFT: u32 = CTRL | SHIFT;
    if event.type_.get_raw() != KeyEventType::RAWKEYDOWN.get_raw() {
        return None;
    }
    let key = event.windows_key_code;
    // The person's own bindings (browser_shortcut.configure) come first.
    let mods = event.modifiers & (CTRL | SHIFT | ALT);
    let key_name = match key {
        0x30..=0x39 | 0x41..=0x5A => Some(char::from(key as u8).to_string()),
        0x70..=0x7B => Some(format!("F{}", key - 0x6F)),
        _ => None,
    };
    if let Some(key_name) = key_name {
        let combo = [(CTRL, "Ctrl"), (ALT, "Alt"), (SHIFT, "Shift")]
            .iter()
            .filter(|(bit, _)| mods & bit != 0)
            .map(|(_, name)| *name)
            .chain(std::iter::once(key_name.as_str()))
            .collect::<Vec<_>>()
            .join("+");
        if let Some(action) = state::settings_snapshot()["shortcuts"][&combo].as_str() {
            return Some((action.to_string(), true));
        }
    }
    let (name, reserved) = match (mods, key) {
        (CTRL, 0x54) => ("new_tab", true),
        (CTRL_SHIFT, 0x54) => ("reopen_tab", true),
        (CTRL, 0x57 | 0x73) => ("close_tab", true),
        (CTRL, 0x09 | 0x22) => ("next_tab", true),
        (CTRL_SHIFT, 0x09) | (CTRL, 0x21) => ("prev_tab", true),
        (CTRL, 0x31..=0x39) => return Some((format!("tab_{}", key - 0x30), true)),
        (CTRL, 0x4C) | (ALT, 0x44) | (0, 0x75) => ("focus_address", false),
        (CTRL, 0x52) | (0, 0x74) => ("reload", false),
        (CTRL_SHIFT, 0x52) | (CTRL, 0x74) => ("hard_reload", false),
        (ALT, 0x25) => ("back", false),
        (ALT, 0x27) => ("forward", false),
        (CTRL, 0x4B) => ("palette", false),
        (CTRL, 0x48) => ("history", false),
        (CTRL, 0x4A) => ("downloads", false),
        (CTRL, 0x44) => ("bookmark", false),
        (CTRL_SHIFT, 0x42) => ("favorites_bar", false),
        (CTRL, 0x46) | (0, 0x72) => ("find", false),
        (CTRL, 0x50) => ("print", false),
        (CTRL, 0xBB | 0x6B) | (CTRL_SHIFT, 0xBB) => ("zoom_in", true),
        (CTRL, 0xBD | 0x6D) => ("zoom_out", true),
        (CTRL, 0x30 | 0x60) => ("zoom_reset", true),
        _ => return None,
    };
    Some((name.to_string(), reserved))
}

cef::wrap_keyboard_handler! {
    struct AgentGoKeyboardHandler {
        content: bool,
    }

    impl KeyboardHandler {
        fn on_pre_key_event(
            &self,
            _browser: Option<&mut Browser>,
            event: Option<&KeyEvent>,
            os_event: Option<&mut sys::MSG>,
            is_keyboard_shortcut: Option<&mut i32>,
        ) -> i32 {
            let Some((name, reserved)) = event.and_then(browser_shortcut) else {
                // While an agent acts, the page does not take the person's
                // keys. Their keys carry a Windows message; the agent's own
                // (DevTools) keys do not, so those still reach the page.
                return i32::from(self.content && os_event.is_some() && state::agent_locked());
            };
            if reserved {
                state::shell_shortcut(&name);
                return 1;
            }
            // The page sees every other shortcut first (Docs uses Ctrl+H and
            // Ctrl+R); `on_key_event` only runs if the page left it alone.
            if let Some(flag) = is_keyboard_shortcut {
                *flag = 1;
            }
            0
        }

        fn on_key_event(
            &self,
            _browser: Option<&mut Browser>,
            event: Option<&KeyEvent>,
            _os_event: Option<&mut sys::MSG>,
        ) -> i32 {
            let Some((name, _)) = event.and_then(browser_shortcut) else {
                return 0;
            };
            state::shell_shortcut(&name);
            1
        }
    }
}

cef::wrap_display_handler! {
    struct AgentGoDisplayHandler {
        tab_id: String,
    }

    impl DisplayHandler {
        fn on_title_change(&self, _browser: Option<&mut Browser>, title: Option<&CefString>) {
            state::set_tab_title(&self.tab_id, &title.map(|t| t.to_string()).unwrap_or_default());
        }

        fn on_favicon_urlchange(
            &self,
            _browser: Option<&mut Browser>,
            icon_urls: Option<&mut CefStringList>,
        ) {
            let first = icon_urls.and_then(|urls| {
                let urls: Vec<String> = urls.clone().into_iter().collect();
                urls.into_iter().next()
            });
            state::set_tab_favicon(&self.tab_id, first);
        }

        fn on_address_change(
            &self,
            _browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            _url: Option<&CefString>,
        ) {
            if frame.is_some_and(|frame| frame.is_main() != 0) {
                state::notify_shell_state();
            }
        }
    }
}

cef::wrap_life_span_handler! {
    struct AgentGoLifeSpanHandler;

    impl LifeSpanHandler {
        fn on_before_popup(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            _popup_id: i32,
            target_url: Option<&CefString>,
            _target_frame_name: Option<&CefString>,
            _target_disposition: WindowOpenDisposition,
            user_gesture: i32,
            _popup_features: Option<&PopupFeatures>,
            _window_info: Option<&mut WindowInfo>,
            _client: Option<&mut Option<Client>>,
            _settings: Option<&mut BrowserSettings>,
            _extra_info: Option<&mut Option<DictionaryValue>>,
            _no_javascript_access: Option<&mut i32>,
        ) -> i32 {
            // AgentGo owns every window. A popup becomes a normal tab when a
            // user gesture (or trusted agent click) caused it; script-opened
            // popups without a gesture are blocked.
            let url = target_url.map(|url| url.to_string()).unwrap_or_default();
            if user_gesture != 0 && !url.is_empty() {
                state::open_popup_as_tab(&url);
            } else {
                state::log_event(&format!("AG-RUNTIME popup-blocked {url:?}"));
            }
            1
        }
    }
}

cef::wrap_request_handler! {
    struct AgentGoRequestHandler {
        tab_id: String,
    }

    impl RequestHandler {
        fn on_before_browse(
            &self,
            _browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            request: Option<&mut cef::Request>,
            _user_gesture: i32,
            is_redirect: i32,
        ) -> i32 {
            // A web page must never reach AgentGo's own documents (they carry
            // the native bridge) or local files. Only the New Tab page is
            // allowed under the agentgo:// scheme in a content tab.
            let Some(request) = request else { return 0 };
            let target = CefString::from(&request.url()).to_string();
            let lower = target.trim_start().to_ascii_lowercase();
            let shell_page = lower.starts_with("agentgo:");
            // The New Tab and error pages carry no bridge access.
            let allowed_shell = !shell_page
                || state::is_navigable_url(&target)
                || target.split(['?', '#']).next() == Some("agentgo://shell/error.html");
            let from_web = frame
                .map(|f| CefString::from(&f.url()).to_string())
                .is_some_and(|u| u.starts_with("http://") || u.starts_with("https://"));
            let web_to_file = lower.starts_with("file:") && from_web && is_redirect != 0;
            if !allowed_shell || web_to_file {
                state::log_event(&format!(
                    "AG-SECURITY navigation-blocked {:?}",
                    target.chars().take(120).collect::<String>()
                ));
                return 1;
            }
            0
        }

        fn auth_credentials(
            &self,
            _browser: Option<&mut Browser>,
            origin_url: Option<&CefString>,
            is_proxy: i32,
            host: Option<&CefString>,
            port: i32,
            realm: Option<&CefString>,
            scheme: Option<&CefString>,
            callback: Option<&mut AuthCallback>,
        ) -> i32 {
            let Some(callback) = callback else { return 0 };
            super::dialogs::auth_challenge(
                &self.tab_id,
                origin_url.map(|v| v.to_string()).unwrap_or_default(),
                is_proxy != 0,
                host.map(|v| v.to_string()).unwrap_or_default(),
                port,
                realm.map(|v| v.to_string()).unwrap_or_default(),
                scheme.map(|v| v.to_string()).unwrap_or_default(),
                callback.clone(),
            );
            1
        }

        fn resource_request_handler(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            request: Option<&mut Request>,
            _is_navigation: ::std::os::raw::c_int,
            _is_download: ::std::os::raw::c_int,
            request_initiator: Option<&CefString>,
            _disable_default_handling: Option<&mut ::std::os::raw::c_int>,
        ) -> Option<ResourceRequestHandler> {
            super::adblock::handler_for(request?, request_initiator)
        }

        fn on_render_process_unresponsive(
            &self,
            browser: Option<&mut Browser>,
            _callback: Option<&mut UnresponsiveProcessCallback>,
        ) -> i32 {
            if let Some(browser) = browser {
                state::mark_renderer_unresponsive(browser.identifier());
            } else {
                state::log_event(&format!(
                    "AG-RUNTIME renderer-unresponsive tab={} browserId=unknown",
                    self.tab_id
                ));
            }
            0
        }

        fn on_render_process_responsive(&self, browser: Option<&mut Browser>) {
            if let Some(browser) = browser {
                state::mark_renderer_responsive(browser.identifier());
            }
        }

        fn on_render_process_terminated(
            &self,
            browser: Option<&mut Browser>,
            _status: TerminationStatus,
            error_code: i32,
            error_string: Option<&CefString>,
        ) {
            let browser_id = browser.map(|browser| browser.identifier());
            let error = error_string.map(|value| value.to_string());
            state::record_renderer_termination(browser_id, error_code, error.as_deref());
        }
    }
}

cef::wrap_permission_handler! {
    struct AgentGoPermissionHandler {
        tab_id: String,
    }

    impl PermissionHandler {
        fn on_request_media_access_permission(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            requesting_origin: Option<&CefString>,
            requested_permissions: u32,
            callback: Option<&mut MediaAccessCallback>,
        ) -> i32 {
            let origin = requesting_origin
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown-origin".to_string());
            // Only what the person granted to this site (permission.grant).
            let allowed = super::extras::allowed_media(&origin) & requested_permissions;
            if allowed != 0 && allowed == requested_permissions {
                state::log_event(&format!("AG-RUNTIME permission-granted kind=media tab={} origin={origin:?} mask={allowed}", self.tab_id));
                if let Some(callback) = callback {
                    callback.cont(allowed);
                }
                return 1;
            }
            state::log_event(&format!(
                "AG-RUNTIME permission-asked kind=media tab={} origin={origin:?} mask={requested_permissions}",
                self.tab_id
            ));
            match callback {
                Some(callback) => super::permission::request_media(origin, requested_permissions, callback),
                None => return 0,
            }
            1
        }

        fn on_show_permission_prompt(
            &self,
            _browser: Option<&mut Browser>,
            prompt_id: u64,
            requesting_origin: Option<&CefString>,
            requested_permissions: u32,
            callback: Option<&mut PermissionPromptCallback>,
        ) -> i32 {
            let origin = requesting_origin
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown-origin".to_string());
            let allowed = super::extras::allowed_mask(&origin);
            let granted = requested_permissions != 0 && allowed & requested_permissions == requested_permissions;
            state::log_event(&format!(
                "AG-RUNTIME permission-{} kind=prompt tab={} promptId={prompt_id} origin={origin:?} mask={requested_permissions}",
                if granted { "granted" } else { "asked" },
                self.tab_id
            ));
            match callback {
                Some(callback) if granted => callback.cont(PermissionRequestResult::ACCEPT),
                Some(callback) => super::permission::request_prompt(origin, requested_permissions, callback),
                None => return 0,
            }
            1
        }

        fn on_dismiss_permission_prompt(
            &self,
            _browser: Option<&mut Browser>,
            prompt_id: u64,
            _result: PermissionRequestResult,
        ) {
            state::log_event(&format!(
                "AG-RUNTIME permission-prompt-dismissed tab={} promptId={prompt_id}",
                self.tab_id
            ));
        }
    }
}

cef::wrap_dialog_handler! {
    struct AgentGoDialogHandler {
        tab_id: String,
    }

    impl DialogHandler {
        fn on_file_dialog(
            &self,
            _browser: Option<&mut Browser>,
            mode: FileDialogMode,
            title: Option<&CefString>,
            default_file_path: Option<&CefString>,
            accept_filters: Option<&mut CefStringList>,
            _accept_extensions: Option<&mut CefStringList>,
            _accept_descriptions: Option<&mut CefStringList>,
            callback: Option<&mut FileDialogCallback>,
        ) -> i32 {
            state::record_file_dialog_request(
                &self.tab_id,
                mode.get_raw(),
                title.map(|value| value.to_string()).as_deref(),
            );
            // A picker an agent's page opened waits for the agent (file.dialog_*,
            // inside the granted folders); the person's own picker is Windows'.
            let filters: Vec<String> = accept_filters.map(|list| list.clone().into_iter().collect()).unwrap_or_default();
            match callback {
                Some(callback) if super::sessions::lease_holder(&self.tab_id).is_some() => {
                    super::dialogs::hold_file_dialog(
                        &self.tab_id,
                        mode.get_raw(),
                        title.map(|value| value.to_string()).unwrap_or_default(),
                        default_file_path.map(|value| value.to_string()).unwrap_or_default(),
                        filters,
                        callback.clone(),
                    );
                    1
                }
                _ => 0,
            }
        }
    }
}

cef::wrap_download_handler! {
    struct AgentGoDownloadHandler {}

    impl DownloadHandler {
        fn can_download(
            &self,
            _browser: Option<&mut Browser>,
            _url: Option<&CefString>,
            _request_method: Option<&CefString>,
        ) -> i32 {
            state::downloads_enabled() as i32
        }

        fn on_before_download(
            &self,
            _browser: Option<&mut Browser>,
            download_item: Option<&mut DownloadItem>,
            suggested_name: Option<&CefString>,
            callback: Option<&mut BeforeDownloadCallback>,
        ) -> i32 {
            let (Some(item), Some(callback)) = (download_item, callback) else {
                return 0;
            };
            let suggested = suggested_name
                .map(|name| name.to_string())
                .filter(|name| !name.trim().is_empty())
                .unwrap_or_else(|| format!("download-{}.bin", item.id()));
            let path = state::download_destination(item.id(), &suggested);
            state::record_download_before(item, &suggested, &path);
            let path_string = path.to_string_lossy().to_string();
            callback.cont(Some(&CefString::from(path_string.as_str())), 0);
            1
        }

        fn on_download_updated(
            &self,
            _browser: Option<&mut Browser>,
            download_item: Option<&mut DownloadItem>,
            callback: Option<&mut DownloadItemCallback>,
        ) {
            if let Some(item) = download_item {
                if let Some(callback) = callback {
                    state::remember_download_callback(item.id(), callback);
                }
                state::record_download_update(item);
            }
        }
    }
}

wrap_load_handler! {
    struct SurfaceLoadHandler {
        surface: Surface,
        tab_id: String,
    }

    impl LoadHandler {
        fn on_loading_state_change(
            &self,
            _browser: Option<&mut Browser>,
            _is_loading: i32,
            _can_go_back: i32,
            _can_go_forward: i32,
        ) {
            // `on_load_end` runs while CEF still reports the browser as
            // loading, so the chrome's spinner and back/forward buttons are
            // only correct when refreshed from this callback.
            if matches!(self.surface, Surface::Content) {
                state::notify_shell_state();
            }
        }

        fn on_load_start(
            &self,
            _browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            _transition_type: TransitionType,
        ) {
            if !matches!(self.surface, Surface::Content) {
                return;
            }
            let Some(frame) = frame else { return };
            if frame.is_main() != 0 {
                super::cdp::navigation_started(&self.tab_id);
            }
            super::adblock::inject(frame, true);
            super::sitedark::inject(frame, &state::settings_snapshot());
        }

        fn on_load_end(
            &self,
            browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            status: i32,
        ) {
            let Some(frame) = frame else {
                return;
            };
            if matches!(self.surface, Surface::DevTools) {
                if frame.is_main() != 0 {
                    state::devtools_loaded(browser.as_deref(), frame);
                }
                return;
            }
            if matches!(self.surface, Surface::Content) {
                super::adblock::inject(frame, false);
            }
            if frame.is_main() == 0 {
                return;
            }
            let url = CefString::from(&frame.url()).to_string();
            match self.surface {
                Surface::Shell if agentgo_shell_bridge::is_trusted_shell_url(&url) => {
                    state::log_event(&format!("AG-RUNTIME shell-loaded status={status}"));
                    // The initial native tab may have loaded before
                    // chrome.js finished installing its callback. Push the
                    // authoritative tab/address snapshot once the trusted
                    // chrome document is ready.
                    state::notify_shell_state();
                    state::maybe_first_run();
                    if state::state().options.smoke {
                        state::advance_smoke(SmokeStep::AwaitShell, &url);
                    }
                }
                Surface::Tool if agentgo_shell_bridge::is_trusted_shell_url(&url) => {
                    state::log_event(&format!("AG-RUNTIME tools-loaded status={status}"));
                    state::refresh_tool_surface();
                }
                Surface::Content => on_content_load(status, &url, &self.tab_id),
                Surface::Shell | Surface::Tool | Surface::DevTools => {}
            }
        }

        fn on_load_error(
            &self,
            _browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            _error_code: Errorcode,
            error_text: Option<&CefString>,
            failed_url: Option<&CefString>,
        ) {
            let Some(frame) = frame.filter(|frame| frame.is_main() != 0) else {
                return;
            };
            let url = failed_url.map(|value| value.to_string()).unwrap_or_default();
            let error = error_text.map(|value| value.to_string()).unwrap_or_default();
            state::log_event(&format!("AG-RUNTIME load-error {url} {error}"));
            // A sign-in prompt: Chromium gave up because nothing answered it.
            // AgentGo answers it through DevTools from now on (dialogs.rs).
            if matches!(self.surface, Surface::Content)
                && error == "ERR_INVALID_AUTH_CREDENTIALS"
                && super::dialogs::arm_auth(&self.tab_id, &url)
            {
                return;
            }
            if matches!(self.surface, Surface::Content) && error != "ERR_ABORTED" {
                state::set_load_error(&self.tab_id, &error);
                // Say what happened instead of leaving a blank page.
                if !url.starts_with("agentgo://") {
                    let page = state::error_page_url(&url, &error);
                    frame.load_url(Some(&CefString::from(page.as_str())));
                }
            }
        }
    }
}

fn on_content_load(status: i32, url: &str, tab_id: &str) {
    // CEF reports status 0 for successful file:// navigations because no HTTP
    // response exists. Keep normal HTTP success semantics for remote pages.
    let successful = (url.starts_with("file://") && status == 0) || (200..400).contains(&status);
    if !successful {
        state::log_event(&format!(
            "AG-RUNTIME content-load-non-success status={status} {url}"
        ));
        // An HTTP error page is still a loaded, readable page.
        state::notify_shell_state();
        return;
    }
    state::set_content_loaded_for_tab(tab_id, url);
    state::log_event(&format!(
        "AG-RUNTIME content-loaded tab={tab_id} status={status} {url}"
    ));
    let step = state::state().smoke_step.lock().ok().map(|step| *step);
    match step {
        Some(SmokeStep::AwaitContent) if url.starts_with("http") || url.starts_with("file://") => {
            state::advance_smoke(SmokeStep::AwaitContent, url);
        }
        Some(SmokeStep::AwaitBridgeResult) if url.starts_with("file://") => {
            state::advance_smoke(SmokeStep::AwaitBridgeResult, url);
        }
        _ => {}
    }
}

wrap_browser_view_delegate! {
    struct AgentGoBrowserViewDelegate {
        tab_id: String,
    }

    impl ViewDelegate {
        fn preferred_size(&self, _view: Option<&mut View>) -> Size {
            if self.tab_id == "shell" {
                Size { width: INITIAL_WIDTH, height: state::chrome_height() }
            } else {
                Size {
                    width: INITIAL_WIDTH,
                    height: INITIAL_HEIGHT - state::chrome_height(),
                }
            }
        }
    }

    impl BrowserViewDelegate {
        fn on_browser_created(
            &self,
            browser_view: Option<&mut BrowserView>,
            browser: Option<&mut Browser>,
        ) {
            // A BrowserView created before its parent Window exists can be
            // visible in the CEF view tree while its native child surface is
            // still disabled.  Re-enable it when CEF hands us the browser;
            // this is especially important for the content view, which is
            // attached to the composition Window after creation.
            if let Some(view) = browser_view {
                view.set_enabled(1);
                view.set_visible((self.tab_id != "tools") as i32);
            }
            if let Some(browser) = browser {
                if self.tab_id.starts_with("content-") {
                    state::register_tab_browser_id(&self.tab_id, browser.identifier());
                    super::cdp::attach(&self.tab_id, browser);
                }
                // BrowserView creates its native child surface when the
                // browser callback fires. Reapply the composition at that
                // point as well as after Window::Show(); otherwise a view
                // that was created before the top-level window can retain
                // the initial native bounds and display only its blank
                // background even though navigation succeeds.
                if self.tab_id == "tools" {
                    state::refresh_tool_surface();
                } else {
                    relayout_registered_content();
                }
            }
        }

        fn browser_runtime_style(&self) -> RuntimeStyle {
            // AgentGo owns the visible chrome. Alloy avoids accidentally
            // composing CEF's Chrome runtime UI above the trusted shell.
            RuntimeStyle::ALLOY
        }
    }
}

wrap_window_delegate! {
    struct AgentGoWindowDelegate {
        shell_view: RefCell<Option<BrowserView>>,
        tools_view: RefCell<Option<BrowserView>>,
    }

    impl ViewDelegate {
        fn preferred_size(&self, _view: Option<&mut View>) -> Size {
            Size { width: INITIAL_WIDTH, height: INITIAL_HEIGHT }
        }
    }

    impl PanelDelegate {}

    impl WindowDelegate {
        fn is_frameless(&self, _window: Option<&mut Window>) -> i32 {
            // AgentGo renders its own title/tab strip. CEF's native frame
            // would add a second close bar above it and consume usable space.
            1
        }

        fn can_resize(&self, _window: Option<&mut Window>) -> i32 {
            1
        }

        fn can_maximize(&self, _window: Option<&mut Window>) -> i32 {
            1
        }

        fn can_minimize(&self, _window: Option<&mut Window>) -> i32 {
            1
        }

        fn on_window_created(&self, window: Option<&mut Window>) {
            let (Some(window), Some(shell), Some(tools)) = (
                window,
                self.shell_view.borrow().clone(),
                self.tools_view.borrow().clone(),
            ) else {
                state::log_event("AG-RUNTIME window-missing-child-view");
                state::request_quit(10);
                return;
            };
            state::set_window(window.clone());
            #[cfg(target_os = "windows")]
            super::guard::protect(window.window_handle().0 as windows_sys::Win32::Foundation::HWND);
            apply_window_icon(window);
            state::refresh_window_title();
            let layout_settings = BoxLayoutSettings {
                horizontal: 0,
                cross_axis_alignment: AxisAlignment::STRETCH,
                ..Default::default()
            };
            let Some(layout) = window.set_to_box_layout(Some(&layout_settings)) else {
                state::log_event("AG-RUNTIME window-box-layout-create-failed");
                state::request_quit(10);
                return;
            };
            let mut shell_as_view = View::from(&shell);
            window.add_child_view(Some(&mut shell_as_view));
            // Tabs live in one panel that takes everything below the chrome.
            // They are added once the content request context is ready
            // (state::create_first_tab).
            let Some(host) = panel_create(None) else {
                state::log_event("AG-RUNTIME tab-host-create-failed");
                state::request_quit(10);
                return;
            };
            host.set_to_fill_layout();
            let mut host_as_view = View::from(&host);
            window.add_child_view(Some(&mut host_as_view));
            layout.set_flex_for_view(Some(&mut host_as_view), 1);
            state::set_tab_host(host);

            // Utilities are their own opaque surface and occupy only the
            // drawer/popover rectangle. The active page remains a real sibling
            // below the chrome and stays visible outside that rectangle.
            let mut tools_as_view = View::from(&tools);
            let Some(tool_overlay) =
                window.add_overlay_view(Some(&mut tools_as_view), DockingMode::CUSTOM, 1)
            else {
                state::log_event("AG-RUNTIME tools-overlay-create-failed");
                state::request_quit(10);
                return;
            };
            tool_overlay.set_visible(0);
            tools.set_visible(0);
            state::set_tool_overlay(tool_overlay);
            let (views, tab_ids) = state::main_window_tabs();
            layout_composition(window, &views, &tab_ids, &shell);
            if state::state().options.background() {
                show_without_activation(window);
            } else {
                // The person started this window themselves: it may come forward.
                super::guard::allowing(|| show_in_front(window));
            }

            // Show() performs the first native layout. Re-run it once so
            // BrowserViews created before the Window receive their final
            // native child bounds immediately.
            let (views, tab_ids) = state::main_window_tabs();
            layout_composition(window, &views, &tab_ids, &shell);
            log_composition_state(&shell, views.first());
        }

        fn on_window_bounds_changed(&self, window: Option<&mut Window>, _new_bounds: Option<&Rect>) {
            let (Some(window), Some(shell)) = (window, self.shell_view.borrow().clone()) else {
                return;
            };
            let (views, tab_ids) = state::main_window_tabs();
            layout_composition(window, &views, &tab_ids, &shell);
        }

        fn on_window_destroyed(&self, _window: Option<&mut Window>) {
            *self.shell_view.borrow_mut() = None;
            *self.tools_view.borrow_mut() = None;
            state::clear_ui_handles();
            state::log_event("AG-RUNTIME window-destroyed");
            state::request_quit(0);
        }

        fn can_close(&self, _window: Option<&mut Window>) -> i32 {
            // No unload-bypass logic belongs in the spike. CEF's browser
            // process owns renderer teardown; `on_window_destroyed` requests
            // a clean host shutdown afterwards.
            1
        }

        fn window_runtime_style(&self) -> RuntimeStyle {
            RuntimeStyle::ALLOY
        }
    }
}

/// Shows the window for an agent-started browser: a normal, visible window
/// with its own taskbar button, placed behind everything else and never
/// activated, so it cannot take the keyboard focus or cover what the person
/// is doing. They bring it forward from the taskbar or Alt+Tab to watch.
#[cfg(target_os = "windows")]
fn show_without_activation(window: &Window) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, IsWindowVisible, SetWindowLongPtrW, SetWindowPos, ShowWindow,
        GWL_EXSTYLE, HWND_BOTTOM, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SW_SHOWNOACTIVATE,
        WS_EX_APPWINDOW, WS_EX_NOACTIVATE,
    };
    let hwnd = window.window_handle().0 as windows_sys::Win32::Foundation::HWND;
    if hwnd.is_null() {
        window.show();
        return;
    }
    // The window is shown through Views so that its compositor, its pages and
    // its screenshots come up exactly as for a window the person opened. While
    // it is shown, the no-activate style keeps the operating system from
    // giving it the foreground. The shell decides about the taskbar button and
    // the Alt+Tab entry at that moment and never looks again, and it leaves
    // out every no-activate window that is not also an app window. So the
    // window is marked as an app window first, and only the no-activate style
    // is removed afterwards, so that a click by the person activates it.
    // SAFETY: `hwnd` is the live top-level window CEF just created on this
    // thread; the calls only change its style, show state and z-order.
    unsafe {
        let styles = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) | WS_EX_APPWINDOW as isize;
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, styles | WS_EX_NOACTIVATE as isize);
        // To the back while still hidden, so it never appears in front even
        // for one frame; the guard keeps it there while it is shown.
        let to_back = || {
            super::guard::allowing(|| {
                SetWindowPos(
                    hwnd,
                    HWND_BOTTOM,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                )
            })
        };
        to_back();
        window.show();
        // The first ShowWindow call of a process obeys the STARTUPINFO of
        // whoever launched it, which may ask for a hidden window.
        if IsWindowVisible(hwnd) == 0 {
            ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        }
        to_back();
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, styles);
    }
}

#[cfg(not(target_os = "windows"))]
fn show_without_activation(window: &Window) {
    window.show();
}

wrap_window_delegate! {
    struct SecondaryWindowDelegate {
        id: String,
        bounds: Rect,
    }

    impl ViewDelegate {
        fn preferred_size(&self, _view: Option<&mut View>) -> Size {
            Size { width: self.bounds.width.max(320), height: self.bounds.height.max(240) }
        }
    }

    impl PanelDelegate {}

    impl WindowDelegate {
        fn initial_bounds(&self, _window: Option<&mut Window>) -> Rect {
            self.bounds.clone()
        }

        fn can_resize(&self, _window: Option<&mut Window>) -> i32 {
            1
        }

        fn can_maximize(&self, _window: Option<&mut Window>) -> i32 {
            1
        }

        fn can_minimize(&self, _window: Option<&mut Window>) -> i32 {
            1
        }

        fn can_close(&self, _window: Option<&mut Window>) -> i32 {
            // Its tabs go back to the main window instead of closing.
            super::winmgr::closing(&self.id);
            1
        }

        fn on_window_created(&self, window: Option<&mut Window>) {
            let Some(window) = window else { return };
            #[cfg(target_os = "windows")]
            super::guard::protect(window.window_handle().0 as windows_sys::Win32::Foundation::HWND);
            apply_window_icon(window);
            let Some(host) = panel_create(None) else {
                state::log_event("AG-RUNTIME secondary-host-create-failed");
                return;
            };
            host.set_to_fill_layout();
            window.set_to_fill_layout();
            let mut as_view = View::from(&host);
            window.add_child_view(Some(&mut as_view));
            super::winmgr::created(&self.id, window.clone(), host);
            show_without_activation(window);
        }

        fn on_window_destroyed(&self, _window: Option<&mut Window>) {
            super::winmgr::destroyed(&self.id);
        }
    }
}

/// Opens another AgentGo window (`window.create`, a detached tab, a popup)
/// without taking the foreground. Its content arrives through `winmgr`.
pub(crate) fn create_secondary_window(id: &str, bounds: Rect) -> bool {
    let mut delegate = SecondaryWindowDelegate::new(id.to_string(), bounds);
    window_create_top_level(Some(&mut delegate)).is_some()
}

/// Brings one of AgentGo's windows forward on the person's request.
pub(crate) fn bring_forward(window: &Window) {
    super::guard::allowing(|| {
        window.show();
        window.activate();
    });
}

/// Shows and activates the window for a launch the person made themselves.
fn show_in_front(window: &Window) {
    // Chromium activates the page a moment after the window is shown.
    super::guard::allow_for(3_000);
    window.show();
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            IsWindowVisible, SetForegroundWindow, ShowWindow, SW_SHOW,
        };
        let hwnd = window.window_handle().0 as windows_sys::Win32::Foundation::HWND;
        // SAFETY: `hwnd` is the live window CEF just created on this thread.
        // A launcher that asked for a hidden first window swallows the show.
        if !hwnd.is_null() && unsafe { IsWindowVisible(hwnd) } == 0 {
            unsafe { ShowWindow(hwnd, SW_SHOW) };
        }
        // The person launched it: it opens in front, ready for them.
        window.activate();
        if !hwnd.is_null() {
            use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
                keybd_event, KEYEVENTF_KEYUP, VK_MENU,
            };
            // SAFETY: as above. Windows refuses SetForegroundWindow to a
            // process that was not the last to receive input (a launcher, a
            // terminal); a synthetic Alt press lifts that restriction, and the
            // key goes to no window.
            unsafe {
                keybd_event(VK_MENU as u8, 0, 0, 0);
                keybd_event(VK_MENU as u8, 0, KEYEVENTF_KEYUP, 0);
                SetForegroundWindow(hwnd);
            }
        }
    }
}

/// Adds `value` to a comma-separated Chromium switch such as
/// `--disable-features`, keeping what is already there.
#[cfg(target_os = "windows")]
fn merge_switch_list(command_line: &mut CommandLine, name: &str, value: &str) {
    let switch = CefString::from(name);
    let existing = if command_line.has_switch(Some(&switch)) != 0 {
        CefString::from(&command_line.switch_value(Some(&switch))).to_string()
    } else {
        String::new()
    };
    if existing.split(',').any(|item| item == value) {
        return;
    }
    let merged = if existing.is_empty() {
        value.to_string()
    } else {
        format!("{existing},{value}")
    };
    command_line.append_switch_with_value(Some(&switch), Some(&CefString::from(merged.as_str())));
}

/// Gives the window (taskbar, Alt+Tab) the AgentGo mark. The PNGs are
/// generated by `scripts/dev/build-icon.py`; the executable carries the same
/// artwork as its file icon (see the app crate's `build.rs`).
fn apply_window_icon(window: &Window) {
    const ICONS: [(f32, &[u8]); 4] = [
        (1.0, include_bytes!("../../assets/icon-32.png")),
        (2.0, include_bytes!("../../assets/icon-64.png")),
        (4.0, include_bytes!("../../assets/icon-128.png")),
        (8.0, include_bytes!("../../assets/icon-256.png")),
    ];
    let Some(mut image) = image_create() else {
        return;
    };
    for (scale, png) in ICONS {
        image.add_png(scale, Some(png));
    }
    window.set_window_icon(Some(&mut image));
    window.set_window_app_icon(Some(&mut image));
}

fn layout_composition(
    window: &Window,
    content_views: &[BrowserView],
    tab_ids: &[String],
    shell: &BrowserView,
) {
    shell.set_visible(1);
    // Every tab stays shown at full size and the active one is stacked on
    // top. A hidden view has no size and no compositor, so an agent could not
    // click, read layout or take a picture in a tab that is not on screen.
    // ponytail: every open tab keeps rendering; hide tabs no session has
    // touched for a while if memory or GPU use becomes a problem.
    let active_id = state::active_tab_id();
    for (index, content) in content_views.iter().enumerate() {
        content.set_visible(1);
        if tab_ids.get(index) == Some(&active_id) {
            // A no-op when the view already is the last child.
            if let Some(host) = state::tab_host() {
                let mut as_view = View::from(content);
                host.reorder_child_view(Some(&mut as_view), -1);
            }
        }
    }
    window.layout();
    for content in content_views {
        if let Some(browser) = content.browser() {
            if let Some(host) = browser.host() {
                host.notify_move_or_resize_started();
            }
        }
    }
    state::relayout_tool_surface();
}

/// Records the native Views state once after the initial show. Load callbacks
/// can report HTTP success even when a child View has been laid out at zero
/// size or is covered by a sibling, so this keeps the visual composition
/// diagnosable without relying on a screen-scraping test.
fn log_composition_state(shell: &BrowserView, content: Option<&BrowserView>) {
    let shell_bounds = shell.bounds();
    let content_state = content.map(|view| {
        let bounds = view.bounds();
        let screen = view.bounds_in_screen();
        format!(
            "content={}x{}+{},{} screen={}x{}+{},{} visible={} drawn={} attached={}",
            bounds.width,
            bounds.height,
            bounds.x,
            bounds.y,
            screen.width,
            screen.height,
            screen.x,
            screen.y,
            view.is_visible(),
            view.is_drawn(),
            view.is_attached()
        )
    });
    state::log_event(&format!(
        "AG-RUNTIME composition-state shell={}x{}+{},{} visible={} drawn={} attached={} {}",
        shell_bounds.width,
        shell_bounds.height,
        shell_bounds.x,
        shell_bounds.y,
        shell.is_visible(),
        shell.is_drawn(),
        shell.is_attached(),
        content_state.unwrap_or_else(|| "content=missing".to_string())
    ));
}
