// `agentgo` executable entry point. It is also compiled as `agentgo-tool`
// (src/bin/agentgo-tool.rs includes this file) so repository tooling works
// without CEF; a comment, not `//!`, because an included file cannot carry
// inner documentation.

use agentgo_cli::{Cli, Command, ManifestAction, ProbeAction};
use clap::Parser;

fn repo_root() -> std::path::PathBuf {
    // Tests/CI run from the workspace; the binary resolves the repo root
    // from its own location when installed inside target/.
    if let Ok(dir) = std::env::var("AGENTGO_REPO") {
        return std::path::PathBuf::from(dir);
    }
    let mut dir = std::env::current_exe().unwrap_or_default();
    // A packaged copy carries its interface next to the executable.
    if dir
        .parent()
        .is_some_and(|folder| folder.join("ui/chrome.html").exists())
    {
        dir.pop();
        return dir;
    }
    for _ in 0..6 {
        dir.pop();
        if dir.join("docs/capability-inventory.md").exists() {
            return dir;
        }
    }
    std::env::current_dir().unwrap_or_default()
}

/// When this is a browser launch by the person (no subcommand, or `run`
/// without --stdio/--smoke) and a console is attached (a double-click, a
/// shortcut, a batch file or a terminal), starts the same command again
/// with no console window and exits, so no terminal window stays open for
/// as long as the browser runs.
#[cfg(all(windows, feature = "runtime"))]
fn relaunch_without_console() {
    use std::os::windows::process::CommandExt;
    use windows_sys::Win32::System::Console::GetConsoleProcessList;
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    let browser = args.first().is_none_or(|first| first == "run")
        && !args.iter().any(|arg| arg == "--stdio" || arg == "--smoke");
    if !browser || std::env::var_os("AGENTGO_NO_CONSOLE").is_some() {
        return;
    }
    let mut list = [0u32; 4];
    // SAFETY: the buffer is valid for its length; the call only writes to it.
    let attached = unsafe { GetConsoleProcessList(list.as_mut_ptr(), list.len() as u32) };
    // A windows-subsystem program started by cmd.exe has no console of its
    // own but keeps cmd's console as its standard output, and cmd waits for
    // it: that window would stay open as long as the browser does.
    // SAFETY: plain handle queries.
    let console_output = unsafe {
        use windows_sys::Win32::Storage::FileSystem::GetFileType;
        use windows_sys::Win32::System::Console::{GetStdHandle, STD_OUTPUT_HANDLE};
        GetFileType(GetStdHandle(STD_OUTPUT_HANDLE)) == 2 // FILE_TYPE_CHAR
    };
    if attached == 0 && !console_output {
        return;
    }
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let started = std::process::Command::new(exe)
        .args(&args)
        .env("AGENTGO_NO_CONSOLE", "1")
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .spawn();
    if started.is_ok() {
        std::process::exit(0);
    }
}

/// Registers this install with an agent host under the full path of
/// `agentgo.exe`, so it does not depend on the host's PATH.
fn setup_host(host: &str) -> i32 {
    let current = std::env::current_exe().unwrap_or_default();
    let sibling = current.with_file_name("agentgo.exe");
    let exe = if sibling.is_file() { sibling } else { current };
    let exe = exe
        .to_string_lossy()
        .trim_start_matches(r"\\?\")
        .to_string();
    if host == "json" {
        println!(
            "{}",
            serde_json::json!({"mcpServers": {"agentgo": {"command": exe, "args": ["mcp"]}}})
        );
        return 0;
    }
    let run = |args: &[&str]| {
        std::process::Command::new("cmd")
            .arg("/C")
            .arg(host)
            .args(args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    };
    // Replace an older entry (possibly with a wrong path) rather than fail.
    // Claude Code's default scope is the current folder only; AgentGo is
    // registered for the user so every project sees it.
    let added = if host == "claude" {
        run(&["mcp", "remove", "--scope", "user", "agentgo"]);
        run(&["mcp", "remove", "agentgo"]);
        run(&[
            "mcp", "add", "--scope", "user", "agentgo", "--", &exe, "mcp",
        ])
    } else if host == "codex" {
        run(&["mcp", "remove", "agentgo"]);
        run(&["mcp", "add", "agentgo", "--", &exe, "mcp"])
    } else {
        // Droid's command line treats backslashes as escapes and would store
        // `C:Usersme...`; Windows takes forward slashes just as well.
        run(&["mcp", "remove", "agentgo"]);
        run(&["mcp", "add", "agentgo", &exe.replace('\\', "/"), "mcp"])
    };
    if added {
        println!("registered AgentGo with {host}: {exe} mcp");
        println!("it attaches to the open AgentGo browser, or starts one if none is open");
        0
    } else {
        eprintln!("could not run `{host} mcp add`. Add this by hand:");
        let scope = if host == "claude" {
            " --scope user"
        } else {
            ""
        };
        let dashes = if host == "claude" || host == "codex" {
            " --"
        } else {
            ""
        };
        eprintln!("  {host} mcp add{scope} agentgo{dashes} \"{exe}\" mcp");
        1
    }
}

fn main() {
    // CEF child processes re-enter this executable with Chromium flags such
    // as `--type=renderer`. They must run before clap sees those flags, or
    // every renderer/GPU process exits with "unexpected argument --type".
    #[cfg(feature = "runtime")]
    if std::env::args_os()
        .skip(1)
        .any(|arg| arg.to_string_lossy().starts_with("--type"))
    {
        if let Some(code) = agentgo_cef::runtime::try_execute_subprocess() {
            std::process::exit(code);
        }
    }

    // agentgo.exe is a console program (agents and the command line use its
    // stdin and stdout). Started by the person from Explorer or a shortcut,
    // Windows gives it a console window of its own: start again without one.
    // agentgo-browser.exe (the Start menu shortcut) never opens a console.
    #[cfg(all(windows, feature = "runtime"))]
    relaunch_without_console();

    // A double-clicked runtime executable has no subcommand. Make that the
    // friendly desktop launch path instead of showing a transient Clap error
    // and exiting before the native window can be created.
    if std::env::args_os().nth(1).is_none() {
        #[cfg(feature = "runtime")]
        {
            let root = repo_root();
            agentgo_cef::runtime::run_browser(agentgo_cef::runtime::RunOptions {
                ui_root: root.join("ui"),
                log_path: std::env::temp_dir().join("agentgo-runtime.log"),
                ..Default::default()
            });
        }
        #[cfg(not(feature = "runtime"))]
        {
            eprintln!(
                "this executable was built without the native runtime; use the runtime build or run `agentgo --help`"
            );
            std::process::exit(2);
        }
    }

    let cli = Cli::parse();
    let code = match cli.command {
        Command::Version { json } => {
            let v = agentgo_app::versions_report();
            if json {
                println!("{}", serde_json::to_string_pretty(&v).unwrap());
            } else {
                print!("{}", agentgo_app::Versions::collect().to_text());
            }
            0
        }
        Command::Doctor { json, strict } => {
            let report = agentgo_app::run_doctor(&repo_root());
            if json {
                println!("{}", serde_json::to_string_pretty(&report).unwrap());
            } else {
                println!("AgentGo doctor — {}", report.versions.agentgo);
                for c in &report.checks {
                    let mark = match c.status {
                        agentgo_app::DoctorStatus::Ok => "[ok]  ",
                        agentgo_app::DoctorStatus::Warn => "[warn]",
                        agentgo_app::DoctorStatus::Fail => "[FAIL]",
                    };
                    println!("{mark} {} — {}", c.label, c.detail);
                }
                println!("exit: {}", report.exit_code);
            }
            let mut code = report.exit_code;
            if strict
                && code == 0
                && report
                    .checks
                    .iter()
                    .any(|c| c.status == agentgo_app::DoctorStatus::Warn)
            {
                code = 1;
            }
            code
        }
        Command::Manifest {
            action: ManifestAction::Generate { inventory, out },
        } => match agentgo_app::generate_manifest(&inventory) {
            Ok(json) => {
                if let Some(parent) = out.parent() {
                    std::fs::create_dir_all(parent).ok();
                }
                std::fs::write(&out, &json).expect("write manifest");
                let m: serde_json::Value = serde_json::from_str(&json).unwrap();
                println!(
                    "wrote {} ({} capabilities)",
                    out.display(),
                    m["counts"]["total"]
                );
                0
            }
            Err(e) => {
                eprintln!("manifest generate failed: {e}");
                1
            }
        },
        Command::Manifest {
            action:
                ManifestAction::Validate {
                    manifest,
                    inventory,
                },
        } => match agentgo_app::validate_manifest(&manifest, &inventory) {
            Ok(m) => {
                println!(
                    "manifest ok: {} capabilities ({} P0) across {} sections",
                    m.counts.total,
                    m.counts.p0,
                    m.sections.len()
                );
                0
            }
            Err(e) => {
                eprintln!("manifest validate failed: {e}");
                1
            }
        },
        Command::Call {
            method,
            params,
            instance,
        } => {
            let params = match params
                .as_deref()
                .map(serde_json::from_str::<serde_json::Value>)
            {
                None => serde_json::json!({}),
                Some(Ok(value)) => value,
                Some(Err(error)) => {
                    eprintln!("params must be JSON: {error}");
                    std::process::exit(2);
                }
            };
            match agentgo_mcp::client::Client::attach(instance.as_deref(), "agentgo-cli")
                .and_then(|mut client| client.call(&method, params))
            {
                Ok(result) => {
                    // Page views print as text; everything else as JSON.
                    match result
                        .get("page")
                        .or_else(|| result.get("text"))
                        .and_then(|v| v.as_str())
                    {
                        Some(_) => println!("{}", agentgo_mcp::server::render(&result)),
                        None => println!("{}", serde_json::to_string_pretty(&result).unwrap()),
                    }
                    0
                }
                Err(error) => {
                    eprintln!("{error}");
                    1
                }
            }
        }
        Command::Setup { host } => setup_host(&host),
        Command::Import {
            browser,
            id,
            what,
            csv,
            open_export,
        } => agentgo_app::import::run(
            browser.as_deref(),
            id.as_deref(),
            &what,
            csv.as_deref(),
            open_export.as_deref(),
        ),
        Command::Mcp {
            attach,
            fresh,
            profile,
            mode,
            allow_downloads,
            upload_dir,
        } => {
            let open = if fresh {
                None
            } else {
                agentgo_mcp::client::Client::attach(None, "mcp").ok()
            };
            let client = if attach {
                agentgo_mcp::client::Client::attach(None, "mcp")
            } else if let Some(client) = open {
                Ok(client)
            } else {
                let mut args = vec!["--url".to_string(), "about:blank".to_string()];
                if let Some(profile) = profile {
                    args.extend(["--profile".into(), profile]);
                }
                if let Some(mode) = mode {
                    args.extend(["--mode".into(), mode]);
                }
                if allow_downloads {
                    args.push("--allow-downloads".into());
                }
                for dir in upload_dir {
                    args.extend(["--upload-dir".into(), dir.to_string_lossy().into_owned()]);
                }
                let exe = std::env::current_exe().unwrap_or_default();
                // `agentgo-tool` has no browser inside; use its sibling.
                let browser = exe.with_file_name("agentgo.exe");
                agentgo_mcp::client::Client::launch(
                    if browser.exists() { &browser } else { &exe },
                    &args,
                )
            };
            match client {
                Ok(client) => {
                    let stdin = std::io::stdin();
                    agentgo_mcp::server::serve(client, stdin.lock(), std::io::stdout()).close();
                    0
                }
                Err(error) => {
                    eprintln!("{error}");
                    1
                }
            }
        }
        Command::Probe { action } => match action {
            ProbeAction::Windows => {
                println!(
                    "{}",
                    serde_json::to_string(&agentgo_app::probe::windows_json()).unwrap()
                );
                0
            }
            ProbeAction::Watch { seconds, class } => {
                agentgo_app::probe::watch(seconds, &class);
                0
            }
            ProbeAction::Decoy { title, cover } => {
                match agentgo_app::probe::run_decoy(&title, cover) {
                    Ok(()) => 0,
                    Err(error) => {
                        eprintln!("probe decoy: {error}");
                        1
                    }
                }
            }
        },
        Command::QuietRun { report, command } => {
            let (program, args) = command.split_first().expect("clap requires a command");
            match agentgo_app::quiet::run(program, args) {
                Ok(outcome) => {
                    let leaked = !outcome.leaked.is_empty();
                    if report || leaked {
                        eprintln!(
                            "quiet-run: exit {} | {} process(es) | peak {} visible window(s) on the private desktop | {} on your desktop{}",
                            outcome.exit_code,
                            outcome.peak_processes,
                            outcome.peak_private_windows,
                            outcome.leaked.len(),
                            if leaked {
                                format!(" ({})", outcome.leaked.join(", "))
                            } else {
                                String::new()
                            }
                        );
                    }
                    if leaked {
                        agentgo_app::quiet::LEAK_EXIT_CODE
                    } else {
                        outcome.exit_code
                    }
                }
                Err(error) => {
                    eprintln!("quiet-run: {error}");
                    1
                }
            }
        }
        Command::Run {
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
        } => {
            #[cfg(feature = "runtime")]
            {
                let root = repo_root();
                agentgo_cef::runtime::run_browser(agentgo_cef::runtime::RunOptions {
                    content_url: url.unwrap_or_default(),
                    smoke,
                    stdio,
                    profile_id: profile.unwrap_or_default(),
                    ephemeral,
                    allow_downloads,
                    allow_evaluate,
                    control_mode: mode,
                    upload_dirs: upload_dir,
                    foreground,
                    restore,
                    ui_root: root.join("ui"),
                    log_path: std::env::temp_dir().join("agentgo-runtime.log"),
                    ..Default::default()
                });
            }
            #[cfg(not(feature = "runtime"))]
            {
                let _ = (
                    profile,
                    ephemeral,
                    allow_downloads,
                    allow_evaluate,
                    mode,
                    upload_dir,
                    foreground,
                    restore,
                );
                eprintln!(
                    "agentgo run needs the native runtime build. Run: \
                 cargo run -p agentgo-app --features runtime -- run \
                  (url={url:?} smoke={smoke} stdio={stdio})"
                );
                2
            }
        }
    };
    std::process::exit(code);
}
