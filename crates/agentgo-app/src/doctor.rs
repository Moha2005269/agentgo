//! Environment doctor: verifies the pinned toolchain, local layout, and
//! manifest consistency. Used by `agentgo doctor` and CI.

use std::path::PathBuf;
use std::process::Command;

use serde::Serialize;

use crate::manifest;
use crate::versions::Versions;
use agentgo_cef as cef_pins;
use agentgo_storage::AgentGoPaths;

/// One check result.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DoctorCheck {
    /// Stable check id.
    pub id: &'static str,
    /// What was checked.
    pub label: String,
    /// Outcome.
    pub status: DoctorStatus,
    /// Non-secret detail.
    pub detail: String,
}

/// Check outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DoctorStatus {
    /// Pass.
    Ok,
    /// Warning (does not fail doctor unless `--strict`).
    Warn,
    /// Failure.
    Fail,
}

/// Full doctor report.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DoctorReport {
    /// Versions snapshot.
    pub versions: Versions,
    /// Checks in execution order.
    pub checks: Vec<DoctorCheck>,
    /// Non-zero when any check failed.
    pub exit_code: i32,
}

fn tool_version(exe: &str, args: &[&str]) -> Option<String> {
    Command::new(exe)
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn check(id: &'static str, label: &str, status: DoctorStatus, detail: String) -> DoctorCheck {
    DoctorCheck {
        id,
        label: label.into(),
        status,
        detail,
    }
}

/// Runs all doctor checks against the repository at `repo_root`.
pub fn run_doctor(repo_root: &std::path::Path) -> DoctorReport {
    let mut checks = Vec::new();

    // 1. rustc present and matching the pin.
    match tool_version("rustc", &["--version"]) {
        Some(v) => {
            let pinned = Versions::collect().pinned_rust;
            if v.contains(pinned) {
                checks.push(check("rustc", "Rust toolchain", DoctorStatus::Ok, v));
            } else {
                checks.push(check(
                    "rustc",
                    "Rust toolchain",
                    DoctorStatus::Warn,
                    format!("{v} (rust-toolchain.toml pins {pinned}; run bootstrap to install)"),
                ));
            }
        }
        None => checks.push(check(
            "rustc",
            "Rust toolchain",
            DoctorStatus::Fail,
            "rustc not found on PATH; run scripts/bootstrap.ps1 -Stage toolchain".into(),
        )),
    }

    // 2-5. Build tooling used by the CEF wrapper build.
    for (id, label, exe, args) in [
        ("cmake", "CMake", "cmake", &["--version"][..]),
        ("ninja", "Ninja", "ninja", &["--version"][..]),
        ("git", "Git", "git", &["--version"][..]),
        ("python", "Python", "python", &["--version"][..]),
    ] {
        match tool_version(exe, args) {
            Some(v) => checks.push(check(id, label, DoctorStatus::Ok, v)),
            None => checks.push(check(
                id,
                label,
                DoctorStatus::Warn,
                format!("{exe} not found on PATH"),
            )),
        }
    }

    // 6. MSVC toolchain (needed for the CEF C++ wrapper).
    let vswhere = std::path::Path::new(&std::env::var("ProgramFiles(x86)").unwrap_or_default())
        .join("Microsoft Visual Studio/Installer/vswhere.exe");
    let msvc = if vswhere.exists() {
        Command::new(&vswhere)
            .args([
                "-latest",
                "-products",
                "*",
                "-requires",
                "Microsoft.VisualStudio.Component.VC.Tools.x86.x64",
                "-property",
                "displayName",
            ])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    } else {
        None
    };
    match msvc {
        Some(name) => checks.push(check(
            "msvc",
            "MSVC toolchain (VS C++ tools)",
            DoctorStatus::Ok,
            name,
        )),
        None => checks.push(check(
            "msvc",
            "MSVC toolchain (VS C++ tools)",
            DoctorStatus::Warn,
            "Visual Studio C++ tools not detected; required from Milestone 1 (CEF wrapper build)"
                .into(),
        )),
    }

    // 7. CEF distribution (optional at M0, required from M1).
    let cef_dir_env = std::env::var("CEF_PATH").ok().map(PathBuf::from);
    let cef_dir = cef_dir_env
        .map(|p| p.join(cef_pins::CEF_VERSION).join("windows64"))
        .unwrap_or_else(|| {
            repo_root
                .join("third_party/cef-dist")
                .join(cef_pins::CEF_VERSION)
                .join("windows64")
        });
    match agentgo_cef::verify_distribution(&cef_dir) {
        Ok(report) => checks.push(check(
            "cef-distro",
            "Pinned CEF binary distribution",
            DoctorStatus::Ok,
            format!(
                "CEF {} / Chromium {} at {}",
                report.cef_version, report.chromium_version, report.root
            ),
        )),
        Err(e) => checks.push(check(
            "cef-distro",
            "Pinned CEF binary distribution",
            DoctorStatus::Warn,
            format!("{e}; run scripts/bootstrap.ps1 -Stage cef"),
        )),
    }

    // 8. Local data layout is creatable/writable.
    match AgentGoPaths::system_default() {
        Ok(paths) => match paths.ensure_layout() {
            Ok(()) => checks.push(check(
                "local-layout",
                "Local data layout",
                DoctorStatus::Ok,
                format!("writable at {}", paths.root.display()),
            )),
            Err(e) => checks.push(check(
                "local-layout",
                "Local data layout",
                DoctorStatus::Fail,
                format!("cannot create {}: {e}", paths.root.display()),
            )),
        },
        Err(e) => checks.push(check(
            "local-layout",
            "Local data layout",
            DoctorStatus::Fail,
            e.to_string(),
        )),
    }

    // 9. Capability manifest is in sync with the inventory + versions.
    let inv = repo_root.join("docs/capability-inventory.md");
    let man = repo_root.join("schemas/capability-manifest.json");
    match manifest::validate_manifest(&man, &inv) {
        Ok(m) => checks.push(check(
            "manifest",
            "Capability manifest sync",
            DoctorStatus::Ok,
            format!(
                "{} capabilities ({} P0) across {} sections; protocol {} skill {}",
                m.counts.total,
                m.counts.p0,
                m.sections.len(),
                m.protocol_version,
                m.skill_version
            ),
        )),
        Err(e) => checks.push(check(
            "manifest",
            "Capability manifest sync",
            DoctorStatus::Fail,
            format!("{e}"),
        )),
    }

    let exit_code = if checks.iter().any(|c| c.status == DoctorStatus::Fail) {
        1
    } else {
        0
    };
    DoctorReport {
        versions: Versions::collect(),
        checks,
        exit_code,
    }
}
