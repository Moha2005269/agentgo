//! AgentGo command-line surface.
//!
//! The `agentgo` executable (agentgo-app) uses this crate for parsing and
//! dispatch so the CLI, the current stdio protocol, and the
//! shell share one command vocabulary.
//!
//! The manifest and browser `run` commands are available alongside the
//! repository diagnostics commands; named-pipe control remains later work.

use clap::{Parser, Subcommand};

/// The agentgo CLI definition.
#[derive(Debug, Parser)]
#[command(
    name = "agentgo",
    version,
    about = "AgentGo — the agent-first browser (local control surface)",
    long_about = "AgentGo executable. Agent hosts launch this binary and drive the browser through the local, versioned control protocol; humans use the same binary to launch the visible browser."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

/// Subcommands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Report pinned versions: AgentGo, protocol, CEF/Chromium, cef-rs,
    /// skill, schema, and build facts.
    Version {
        /// Emit machine-readable JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Verify the local environment against the pinned toolchain and
    /// report each check.
    Doctor {
        /// Emit machine-readable JSON instead of text.
        #[arg(long)]
        json: bool,
        /// Treat warnings as failures.
        #[arg(long)]
        strict: bool,
    },
    /// Capability manifest operations.
    Manifest {
        #[command(subcommand)]
        action: ManifestAction,
    },
    /// Send one protocol command to the running AgentGo browser over its
    /// named pipe and print the JSON result.
    Call {
        /// Protocol method, e.g. `observe.page` or `navigate.url`.
        method: String,
        /// JSON parameters, e.g. '{"url":"example.com"}'.
        params: Option<String>,
        /// Target a specific instance id instead of the newest one.
        #[arg(long)]
        instance: Option<String>,
    },
    /// Register AgentGo as an MCP server with an agent host (droid, claude,
    /// codex), using this program's full path. `json` just prints the entry.
    Setup {
        /// droid, claude, codex or json.
        #[arg(value_parser = ["droid", "claude", "codex", "json"])]
        host: String,
    },
    /// Run an MCP server on stdio for agent hosts such as Claude Code. It
    /// attaches to the browser that is already open and starts its own
    /// isolated one only when none is.
    Mcp {
        /// Only attach to the open browser; fail when there is none.
        #[arg(long)]
        attach: bool,
        /// Always start a separate, isolated browser.
        #[arg(long = "new")]
        fresh: bool,
        /// Named profile for the launched browser (keeps logins).
        #[arg(long)]
        profile: Option<String>,
        /// Control mode for the launched browser.
        #[arg(long, value_parser = ["approval", "full"])]
        mode: Option<String>,
        /// Let the agent download files in the launched browser.
        #[arg(long)]
        allow_downloads: bool,
        /// A directory the agent may upload files from (repeatable).
        #[arg(long = "upload-dir")]
        upload_dir: Vec<std::path::PathBuf>,
    },
    /// Run a command on a private Win32 desktop so nothing it opens can
    /// appear on screen, take focus, or receive the user's input. Used for
    /// every real-browser test. Fails (exit 97) if a window of the command
    /// shows up on the user's own desktop.
    QuietRun {
        /// Print what the private desktop saw when the command ends.
        #[arg(long)]
        report: bool,
        /// The command and its arguments (after `--`).
        #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
    },
    /// Window probes for the presentation tests. Run them under `quiet-run`
    /// so they only ever see the private desktop.
    Probe {
        #[command(subcommand)]
        action: ProbeAction,
    },
    /// Launch the AgentGo browser.
    Run {
        /// Initial URL for the untrusted content surface (defaults to
        /// Google).
        #[arg(long)]
        url: Option<String>,
        /// Run the smoke scenario and exit with a report (used by CI).
        #[arg(long)]
        smoke: bool,
        /// Keep stdout as newline-delimited JSON for an external agent and
        /// accept live control requests on stdin.
        #[arg(long)]
        stdio: bool,
        /// Open a named local profile for this run. The profile must already
        /// exist; omitted uses the persisted active profile or Default.
        #[arg(long)]
        profile: Option<String>,
        /// Use a throwaway profile that is deleted when the browser exits.
        #[arg(long)]
        ephemeral: bool,
        /// Operator grant: let agents download files during this run.
        #[arg(long)]
        allow_downloads: bool,
        /// Operator grant: let agents run page script in approval mode.
        #[arg(long)]
        allow_evaluate: bool,
        /// Control mode for this run: `approval` (default) or `full`.
        /// Purchases and payments need approval in both.
        #[arg(long, value_parser = ["approval", "full"])]
        mode: Option<String>,
        /// Operator grant: a directory agents may upload files from
        /// (repeatable).
        #[arg(long = "upload-dir")]
        upload_dir: Vec<std::path::PathBuf>,
        /// Open the window in front and take the keyboard focus even though
        /// an agent started the browser. By default an agent-started window
        /// opens behind your other windows and never takes focus; open it
        /// from the taskbar when you want to watch.
        #[arg(long)]
        foreground: bool,
        /// Reopen the pages of the last session when no --url is given. A
        /// launch without --stdio does this anyway.
        #[arg(long)]
        restore: bool,
    },
}

/// Window probe subcommands.
#[derive(Debug, Subcommand)]
pub enum ProbeAction {
    /// Print the top-level windows of this desktop, front to back, as JSON:
    /// visibility, owner process, and which one is active in its input queue.
    Windows,
    /// Print every change of visibility, activation, focus and stacking of
    /// the windows of one class, with millisecond timestamps.
    Watch {
        /// How long to watch.
        #[arg(long, default_value_t = 10)]
        seconds: u64,
        /// Window class to follow.
        #[arg(long, default_value = "Chrome_WidgetWin_1")]
        class: String,
    },
    /// Open a stand-in window for the person's own window and keep it in
    /// front until the process is stopped.
    Decoy {
        /// Window title.
        #[arg(long, default_value = "AGENTGO-DECOY")]
        title: String,
        /// Cover the whole screen, so any window behind it is fully occluded.
        #[arg(long)]
        cover: bool,
    },
}

/// Manifest subcommands.
#[derive(Debug, Subcommand)]
pub enum ManifestAction {
    /// Regenerate schemas/capability-manifest.json from
    /// docs/capability-inventory.md.
    Generate {
        /// Path to the capability inventory markdown.
        #[arg(long, default_value = "docs/capability-inventory.md")]
        inventory: std::path::PathBuf,
        /// Output path for the manifest.
        #[arg(long, default_value = "schemas/capability-manifest.json")]
        out: std::path::PathBuf,
    },
    /// Validate an existing manifest against the inventory and the
    /// manifest schema rules.
    Validate {
        /// Path to the manifest.
        #[arg(long, default_value = "schemas/capability-manifest.json")]
        manifest: std::path::PathBuf,
        /// Path to the capability inventory markdown.
        #[arg(long, default_value = "docs/capability-inventory.md")]
        inventory: std::path::PathBuf,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn parses_version_json() {
        let cli = Cli::try_parse_from(["agentgo", "version", "--json"]).unwrap();
        assert!(matches!(cli.command, Command::Version { json: true }));
    }

    #[test]
    fn parses_manifest_generate_defaults() {
        let cli = Cli::try_parse_from(["agentgo", "manifest", "generate"]).unwrap();
        match cli.command {
            Command::Manifest {
                action: ManifestAction::Generate { inventory, out },
            } => {
                assert_eq!(
                    inventory,
                    std::path::PathBuf::from("docs/capability-inventory.md")
                );
                assert_eq!(
                    out,
                    std::path::PathBuf::from("schemas/capability-manifest.json")
                );
            }
            other => panic!("expected manifest generate, got {other:?}"),
        }
    }

    #[test]
    fn parses_run_with_url() {
        let cli = Cli::try_parse_from([
            "agentgo",
            "run",
            "--url",
            "https://www.google.com/search?q=agentgo",
        ])
        .unwrap();
        match cli.command {
            Command::Run {
                url,
                smoke,
                stdio,
                profile,
                ..
            } => {
                assert_eq!(
                    url.as_deref(),
                    Some("https://www.google.com/search?q=agentgo")
                );
                assert!(!smoke);
                assert!(!stdio);
                assert!(profile.is_none());
            }
            other => panic!("expected run, got {other:?}"),
        }
    }

    #[test]
    fn quiet_run_keeps_the_command_flags_for_the_command() {
        let cli = Cli::try_parse_from([
            "agentgo-tool",
            "quiet-run",
            "--report",
            "--",
            "node",
            "--test",
            "a.js",
        ])
        .unwrap();
        match cli.command {
            Command::QuietRun { report, command } => {
                assert!(report);
                assert_eq!(command, ["node", "--test", "a.js"]);
            }
            other => panic!("expected quiet-run, got {other:?}"),
        }
        assert!(Cli::try_parse_from(["agentgo-tool", "quiet-run"]).is_err());
    }

    #[test]
    fn rejects_unknown_commands() {
        assert!(Cli::try_parse_from(["agentgo", "bogus"]).is_err());
    }
}
