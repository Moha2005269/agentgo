//! AgentGo executable and composition root.
//!
//! Milestones 0-2 scope: pinned version reporting, environment doctor, and
//! the capability-manifest tool that keeps `docs/capability-inventory.md`,
//! `schemas/capability-manifest.json`, the bundled skill, and tests in
//! lockstep. The CEF browser runtime (`run`) is feature-gated and currently
//! provides the tested Milestones 1-2 foundation.

pub mod doctor;
pub mod manifest;
pub mod probe;
pub mod quiet;
pub mod versions;

pub use doctor::{run_doctor, DoctorReport, DoctorStatus};
pub use manifest::{
    generate_manifest, validate_manifest, ManifestEntry, ManifestError, ManifestSection,
    OWNER_BY_SECTION, PRIORITY_MILESTONE_BY_SECTION,
};
pub use versions::{versions_report, Versions};

/// CEF's Windows bootstrap launcher loads the client as a DLL and calls this
/// exact entry point. Keep the packaged entry separate from the normal Rust
/// executable so a direct development launch can never be mistaken for a
/// sandboxed deployment.
///
/// # Safety
///
/// Called once by `bootstrap.exe` with the sandbox info pointer it owns; the
/// other arguments are unused.
#[cfg(all(target_os = "windows", feature = "sandbox"))]
#[unsafe(no_mangle)]
pub unsafe extern "system" fn RunWinMain(
    _instance: *mut std::ffi::c_void,
    _command_line: *const u16,
    _command_show: i32,
    sandbox_info: *mut std::ffi::c_void,
    _version_info: *mut std::ffi::c_void,
) -> i32 {
    let mut options = sandboxed_options();
    let package_root = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(std::path::Path::to_path_buf))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    options.ui_root = package_root.join("ui");
    options.log_path = std::env::temp_dir().join("agentgo-runtime.log");
    agentgo_cef::runtime::run_browser_sandboxed(options, sandbox_info.cast())
}

/// The packaged `agentgo.exe` is a windows-subsystem program so that opening
/// it never shows a console. When it is run from a terminal for a command
/// (`agentgo call ...`), connect it to that terminal so the output appears.
#[cfg(all(target_os = "windows", feature = "sandbox"))]
fn attach_parent_console() {
    use std::os::windows::io::AsRawHandle;
    #[link(name = "kernel32")]
    extern "system" {
        fn AttachConsole(process_id: u32) -> i32;
        fn GetStdHandle(which: u32) -> *mut core::ffi::c_void;
        fn SetStdHandle(which: u32, handle: *mut core::ffi::c_void) -> i32;
    }
    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    const STD_OUTPUT: u32 = -11i32 as u32;
    const STD_ERROR: u32 = -12i32 as u32;
    // SAFETY: plain Win32 calls; the handle passed to SetStdHandle stays open
    // for the life of the process (it is leaked on purpose).
    unsafe {
        if AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
            return;
        }
        if let Ok(out) = std::fs::OpenOptions::new().write(true).open("CONOUT$") {
            let handle = out.as_raw_handle();
            for which in [STD_OUTPUT, STD_ERROR] {
                let current = GetStdHandle(which);
                if current.is_null() || current as isize == -1 {
                    SetStdHandle(which, handle);
                }
            }
            std::mem::forget(out);
        }
    }
}

/// The launch options a person or agent gave on the command line of
/// `agentgo.exe` (the CEF bootstrap launcher forwards it unchanged), parsed
/// the same way as the plain executable parses `agentgo run ...`. Browser
/// child processes (`--type=...`) and a bare double-click keep the defaults.
#[cfg(all(target_os = "windows", feature = "sandbox"))]
fn sandboxed_options() -> agentgo_cef::runtime::RunOptions {
    use clap::Parser;
    let mut options = agentgo_cef::runtime::RunOptions::default();
    let args: Vec<std::ffi::OsString> = std::env::args_os().collect();
    let child = args
        .iter()
        .any(|arg| arg.to_string_lossy().starts_with("--type"));
    if child || args.len() < 2 {
        return options;
    }
    match agentgo_cli::Cli::try_parse_from(args) {
        Ok(cli) => {
            if let agentgo_cli::Command::Run {
                url,
                smoke,
                stdio,
                profile,
                ephemeral,
                allow_downloads,
                allow_evaluate,
                mode,
                upload_dir,
                foreground,
                restore,
            } = cli.command
            {
                options.content_url = url.unwrap_or_default();
                options.smoke = smoke;
                options.stdio = stdio;
                options.profile_id = profile.unwrap_or_default();
                options.ephemeral = ephemeral;
                options.allow_downloads = allow_downloads;
                options.allow_evaluate = allow_evaluate;
                options.control_mode = mode;
                options.upload_dirs = upload_dir;
                options.foreground = foreground;
                options.restore = restore;
            } else {
                // `call`, `mcp`, `version` and the rest need no browser engine:
                // hand them to the tool that ships beside this executable.
                attach_parent_console();
                let tool = std::env::current_exe()
                    .ok()
                    .map(|exe| exe.with_file_name("agentgo-tool.exe"));
                let code = tool
                    .filter(|tool| tool.is_file())
                    .and_then(|tool| {
                        use std::os::windows::process::CommandExt;
                        std::process::Command::new(tool)
                            .args(std::env::args_os().skip(1))
                            // No console window of its own; the standard
                            // handles of this process are passed on.
                            .creation_flags(0x0800_0000)
                            .status()
                            .ok()
                    })
                    .and_then(|status| status.code())
                    .unwrap_or(2);
                std::process::exit(code);
            }
        }
        Err(error) => {
            let _ = error.print();
            std::process::exit(2);
        }
    }
    options
}
