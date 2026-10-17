//! AgentGo Setup: a small graphical installer and uninstaller.
//!
//! The window is a transparent WebView2 surface; `ui/index.html` draws the
//! floating card. This side does the work: it unpacks the browser (from a
//! package appended to this executable, or the latest GitHub release, both
//! checked against their SHA-256), writes the person's settings, creates
//! shortcuts, the PATH entry and the Windows uninstall entry, registers agent
//! hosts, and runs `agentgo-tool import` for bringing data over from another
//! browser. Everything is per user: no administrator rights.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tao::dpi::LogicalSize;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::platform::windows::{IconExtWindows, WindowBuilderExtWindows};
use tao::window::{Icon, WindowBuilder};
use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_EXPAND_SZ, REG_SZ};
use winreg::{RegKey, RegValue};
use wry::WebViewBuilder;

const REPO: &str = "Moha2005269/agentgo";
/// Trailer of an appended package: sha256 (32) | length (8, LE) | magic (8).
const MAGIC: &[u8; 8] = b"AGPAYLD1";
const TRAILER: u64 = 48;
const NO_WINDOW: u32 = 0x0800_0000;
const DETACHED: u32 = 0x0000_0008;
const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\AgentGo";
const UI: &str = include_str!("../ui/index.html");

static INSTALL_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

enum Ui {
    Eval(String),
    Drag,
    Minimize,
    Close,
    Browse(String),
    PickCsv,
    Exit(i32),
}

type Proxy = EventLoopProxy<Ui>;

fn emit(proxy: &Proxy, value: Value) {
    let _ = proxy.send_event(Ui::Eval(format!(
        "window.__agentgo&&window.__agentgo({value})"
    )));
}

fn log(proxy: &Proxy, text: impl AsRef<str>) {
    emit(proxy, json!({"type": "log", "text": text.as_ref()}));
}

fn stage(proxy: &Proxy, name: &str, state: &str) {
    emit(
        proxy,
        json!({"type": "stage", "stage": name, "state": state}),
    );
}

fn progress(proxy: &Proxy, pct: f64) {
    emit(proxy, json!({"type": "progress", "pct": pct}));
}

fn hidden(program: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut command = Command::new(program);
    command.creation_flags(NO_WINDOW);
    command
}

fn env_path(name: &str) -> PathBuf {
    std::env::var_os(name)
        .map(PathBuf::from)
        .unwrap_or_default()
}

fn data_root() -> PathBuf {
    std::env::var_os("AGENTGO_DATA_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| env_path("LOCALAPPDATA").join("AgentGo"))
}

/// Whether a folder holds, or sits inside, the person's data (profiles,
/// saved sign-ins, accounts, history, settings). Setup, updates and
/// uninstall never move or delete such a folder.
fn touches_user_data(dir: &Path) -> bool {
    let canon = |p: &Path| {
        std::fs::canonicalize(p)
            .unwrap_or_else(|_| p.to_path_buf())
            .to_string_lossy()
            .trim_start_matches(r"\\?\")
            .trim_end_matches('\\')
            .to_ascii_lowercase()
    };
    // A folder with AgentGo's data layout is data, wherever it is.
    let looks_like_data =
        dir.join("config").join("settings.json").is_file() && dir.join("profiles").is_dir();
    let (dir, data) = (canon(dir), canon(&data_root()));
    looks_like_data
        || dir == data
        || data.starts_with(&format!("{dir}\\"))
        || dir.starts_with(&format!("{data}\\"))
}

fn default_dir() -> PathBuf {
    env_path("LOCALAPPDATA").join("Programs").join("AgentGo")
}

fn current_dir() -> PathBuf {
    INSTALL_DIR
        .lock()
        .ok()
        .and_then(|d| d.clone())
        .unwrap_or_else(default_dir)
}

fn registered() -> Option<(PathBuf, String)> {
    let key = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(UNINSTALL_KEY, KEY_READ)
        .ok()?;
    let dir: String = key.get_value("InstallLocation").ok()?;
    let version: String = key.get_value("DisplayVersion").unwrap_or_default();
    Some((PathBuf::from(dir), version))
}

fn host_found(name: &str) -> bool {
    hidden("cmd")
        .args(["/C", "where", name])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn agentgo_running() -> bool {
    hidden("tasklist")
        .args(["/FI", "IMAGENAME eq agentgo.exe", "/NH"])
        .output()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .to_ascii_lowercase()
                .contains("agentgo.exe")
        })
        .unwrap_or(false)
}

/* ------------------------------------------------------------------ */
/* The package                                                        */
/* ------------------------------------------------------------------ */

/// A package appended to this executable: (offset, length, sha256).
fn appended() -> Option<(u64, u64, [u8; 32])> {
    let mut file = File::open(std::env::current_exe().ok()?).ok()?;
    let size = file.metadata().ok()?.len();
    if size < TRAILER {
        return None;
    }
    file.seek(SeekFrom::Start(size - TRAILER)).ok()?;
    let mut trailer = [0u8; TRAILER as usize];
    file.read_exact(&mut trailer).ok()?;
    if &trailer[40..48] != MAGIC {
        return None;
    }
    let mut hash = [0u8; 32];
    hash.copy_from_slice(&trailer[..32]);
    let len = u64::from_le_bytes(trailer[32..40].try_into().ok()?);
    let offset = size.checked_sub(TRAILER + len)?;
    Some((offset, len, hash))
}

/// A window onto part of a file, for reading the appended zip in place.
struct Slice {
    file: File,
    start: u64,
    len: u64,
    pos: u64,
}

impl Read for Slice {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let left = self.len.saturating_sub(self.pos);
        if left == 0 {
            return Ok(0);
        }
        let want = buf.len().min(left as usize);
        self.file.seek(SeekFrom::Start(self.start + self.pos))?;
        let n = self.file.read(&mut buf[..want])?;
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for Slice {
    fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
        let next = match to {
            SeekFrom::Start(n) => n as i64,
            SeekFrom::End(n) => self.len as i64 + n,
            SeekFrom::Current(n) => self.pos as i64 + n,
        };
        if next < 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "seek before start",
            ));
        }
        self.pos = next as u64;
        Ok(self.pos)
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn hash_reader(mut reader: impl Read, mut tick: impl FnMut(u64)) -> std::io::Result<String> {
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        tick(n as u64);
    }
    Ok(hex(&hasher.finalize()))
}

/// The version in a package name such as `AgentGo-0.1.0-win64/`.
fn version_in(name: &str) -> Option<String> {
    let rest = name.split(['/', '\\']).next()?.strip_prefix("AgentGo-")?;
    Some(rest.split("-win64").next()?.to_string())
}

fn appended_version() -> Option<String> {
    let (offset, len, _) = appended()?;
    let file = File::open(std::env::current_exe().ok()?).ok()?;
    let mut zip = zip::ZipArchive::new(Slice {
        file,
        start: offset,
        len,
        pos: 0,
    })
    .ok()?;
    let first = zip.by_index(0).ok()?.name().to_string();
    version_in(&first)
}

struct Release {
    version: String,
    zip: String,
    sha: Option<String>,
}

fn latest_release() -> Result<Release, String> {
    let body = ureq::get(&format!(
        "https://api.github.com/repos/{REPO}/releases/latest"
    ))
    .set("User-Agent", "AgentGo-Setup")
    .set("Accept", "application/vnd.github+json")
    .timeout(std::time::Duration::from_secs(20))
    .call()
    .map_err(|e| format!("GitHub did not answer ({e})"))?
    .into_string()
    .map_err(|e| e.to_string())?;
    let json: Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    let assets = json["assets"].as_array().cloned().unwrap_or_default();
    let url = |suffix: &str| {
        assets
            .iter()
            .find(|a| a["name"].as_str().is_some_and(|n| n.ends_with(suffix)))
            .and_then(|a| a["browser_download_url"].as_str().map(str::to_string))
    };
    let zip = url("-win64.zip").ok_or("the latest release has no Windows package")?;
    Ok(Release {
        version: json["tag_name"]
            .as_str()
            .unwrap_or("")
            .trim_start_matches('v')
            .to_string(),
        zip,
        sha: url("-win64.zip.sha256"),
    })
}

/* ------------------------------------------------------------------ */
/* Install                                                            */
/* ------------------------------------------------------------------ */

fn install(proxy: &Proxy, options: &Value) -> Result<String, String> {
    let dir = PathBuf::from(options["dir"].as_str().unwrap_or("").trim());
    if dir.as_os_str().is_empty() || !dir.is_absolute() {
        return Err(
            "Choose a full folder path, like C:\\Users\\you\\AppData\\Local\\Programs\\AgentGo."
                .into(),
        );
    }
    // An existing folder is replaced only when it is an AgentGo install:
    // never move or delete a folder that holds someone's other files.
    let holds_other_files =
        std::fs::read_dir(&dir).is_ok_and(|mut entries| entries.next().is_some());
    let is_agentgo = dir.join("agentgo.exe").is_file() || dir.join("AgentGo-Setup.exe").is_file();
    if dir.parent().is_none() || (holds_other_files && !is_agentgo) {
        return Err(format!(
            "{} already holds other files. Choose an empty folder, or one named AgentGo.",
            dir.display()
        ));
    }
    if touches_user_data(&dir) {
        return Err(format!(
            "{} holds your AgentGo profile (sign-ins, accounts, history). Choose another folder for the program.",
            dir.display()
        ));
    }
    if agentgo_running() {
        return Err("AgentGo is running. Close it, then try again.".into());
    }
    if let Ok(mut slot) = INSTALL_DIR.lock() {
        *slot = Some(dir.clone());
    }

    // 1. the package
    stage(proxy, "download", "run");
    let temp = std::env::temp_dir().join(format!("agentgo-setup-{}.zip", std::process::id()));
    let _cleanup = Cleanup(temp.clone());
    let (reader, total, expected): (Box<dyn ReadSeek>, u64, Option<String>) = if let Some((
        offset,
        len,
        hash,
    )) = appended()
    {
        log(proxy, "Using the package inside this installer");
        let file = File::open(std::env::current_exe().map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        (
            Box::new(Slice {
                file,
                start: offset,
                len,
                pos: 0,
            }),
            len,
            Some(hex(&hash)),
        )
    } else {
        let release = latest_release().map_err(|e| {
            format!("{e}. Check your internet connection, or download AgentGo-Setup-full.exe from github.com/{REPO}/releases, which needs no download.")
        })?;
        log(proxy, format!("Downloading AgentGo {}", release.version));
        let expected = match &release.sha {
            Some(url) => ureq::get(url)
                .set("User-Agent", "AgentGo-Setup")
                .call()
                .ok()
                .and_then(|r| r.into_string().ok())
                .and_then(|t| t.split_whitespace().next().map(|h| h.to_ascii_lowercase())),
            None => None,
        };
        let response = ureq::get(&release.zip)
            .set("User-Agent", "AgentGo-Setup")
            .call()
            .map_err(|e| format!("The download failed ({e})"))?;
        let total: u64 = response
            .header("Content-Length")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let mut body = response.into_reader();
        let mut out = File::create(&temp).map_err(|e| e.to_string())?;
        let (mut done, mut buf) = (0u64, vec![0u8; 1 << 18]);
        loop {
            let n = body
                .read(&mut buf)
                .map_err(|e| format!("The download broke off ({e})"))?;
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            done += n as u64;
            if total > 0 {
                progress(proxy, done as f64 / total as f64 * 35.0);
            }
        }
        drop(out);
        let file = File::open(&temp).map_err(|e| e.to_string())?;
        (Box::new(file), done, expected)
    };
    progress(proxy, 35.0);
    stage(proxy, "download", "ok");

    // 2. its checksum
    stage(proxy, "verify", "run");
    let mut reader = reader;
    match expected {
        Some(expected) => {
            let mut read = 0u64;
            let actual = hash_reader(&mut reader, |n| {
                read += n;
                if total > 0 && read % (16 << 20) < (1 << 20) {
                    progress(proxy, 35.0 + read as f64 / total as f64 * 8.0);
                }
            })
            .map_err(|e| e.to_string())?;
            if actual != expected {
                return Err("The package does not match its published checksum. Download AgentGo Setup again.".into());
            }
            log(proxy, format!("SHA-256 {}…  matches", &actual[..16]));
        }
        None => log(
            proxy,
            "No published checksum for this package; the files inside are still checked",
        ),
    }
    reader.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    progress(proxy, 43.0);
    stage(proxy, "verify", "ok");

    // 3. unpack, beside the old copy, which is put back if anything fails
    stage(proxy, "extract", "run");
    let backup = dir.with_extension(format!("old-{}", std::process::id()));
    let had_old = dir.exists();
    if had_old {
        std::fs::rename(&dir, &backup)
            .map_err(|_| "The old AgentGo folder is in use. Close AgentGo and any window showing that folder, then try again.".to_string())?;
    }
    let result = extract(proxy, reader, &dir).and_then(|()| check_sums(proxy, &dir));
    if let Err(error) = result {
        let _ = std::fs::remove_dir_all(&dir);
        if had_old {
            let _ = std::fs::rename(&backup, &dir);
        }
        return Err(error);
    }
    if had_old {
        // Keep the uninstaller copy running from the old folder alive.
        let _ = std::fs::remove_dir_all(&backup);
    }
    progress(proxy, 88.0);
    stage(proxy, "extract", "ok");

    // 4. settings
    stage(proxy, "settings", "run");
    write_settings(&options["settings"])
        .map_err(|e| format!("Your preferences could not be saved ({e})"))?;
    log(proxy, "Preferences saved");
    progress(proxy, 91.0);
    stage(proxy, "settings", "ok");

    // 5. shortcuts and PATH
    stage(proxy, "shortcuts", "run");
    let launcher = if dir.join("agentgo-browser.exe").exists() {
        dir.join("agentgo-browser.exe")
    } else {
        dir.join("agentgo.exe")
    };
    let start_menu = env_path("APPDATA").join(r"Microsoft\Windows\Start Menu\Programs\AgentGo.lnk");
    if options["startMenu"] == true {
        shortcut(&start_menu, &launcher, &dir)?;
        log(proxy, "Start menu entry created");
    } else {
        let _ = std::fs::remove_file(&start_menu);
    }
    if options["desktop"] == true {
        shortcut(&desktop().join("AgentGo.lnk"), &launcher, &dir)?;
        log(proxy, "Desktop shortcut created");
    }
    set_user_path(&dir, options["path"] == true);
    progress(proxy, 94.0);
    stage(proxy, "shortcuts", "ok");

    // 6. agents
    stage(proxy, "agents", "run");
    let tool = dir.join("agentgo-tool.exe");
    let mut connected = Vec::new();
    for host in ["claude", "codex", "droid"] {
        if options[host] != true {
            continue;
        }
        let ok = hidden(&tool)
            .args(["setup", host])
            .output()
            .is_ok_and(|o| o.status.success());
        log(
            proxy,
            if ok {
                format!("Registered with {host}")
            } else {
                format!("Could not register with {host}")
            },
        );
        if ok {
            connected.push(host);
        }
    }
    if options["skill"] == true {
        let target = env_path("USERPROFILE").join(r".claude\skills\agentgo");
        let _ = std::fs::remove_dir_all(&target);
        if copy_dir(&dir.join(r"skills\agentgo"), &target).is_ok() {
            log(proxy, "Agent guide copied to ~/.claude/skills");
        }
    }
    progress(proxy, 97.0);
    stage(proxy, "agents", "ok");

    // 7. Windows
    stage(proxy, "register", "run");
    let version = appended_version()
        .or_else(|| latest_release().ok().map(|r| r.version))
        .unwrap_or_default();
    let setup = dir.join("AgentGo-Setup.exe");
    copy_stub(&setup).map_err(|e| format!("The uninstaller could not be written ({e})"))?;
    register(&dir, &setup, &version)
        .map_err(|e| format!("Windows did not accept the uninstall entry ({e})"))?;
    progress(proxy, 100.0);
    stage(proxy, "register", "ok");

    Ok(match connected.len() {
        0 => "Open AgentGo from Start or your desktop.".to_string(),
        _ => format!(
            "Open AgentGo from Start or your desktop. {} can use it right away.",
            pretty_hosts(&connected)
        ),
    })
}

fn pretty_hosts(hosts: &[&str]) -> String {
    let names: Vec<&str> = hosts
        .iter()
        .map(|h| match *h {
            "claude" => "Claude Code",
            "codex" => "Codex",
            _ => "Droid",
        })
        .collect();
    match names.len() {
        1 => names[0].to_string(),
        _ => format!(
            "{} and {}",
            names[..names.len() - 1].join(", "),
            names[names.len() - 1]
        ),
    }
}

trait ReadSeek: Read + Seek {}
impl<T: Read + Seek> ReadSeek for T {}

struct Cleanup(PathBuf);
impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn extract(proxy: &Proxy, reader: Box<dyn ReadSeek>, dir: &Path) -> Result<(), String> {
    let mut zip =
        zip::ZipArchive::new(reader).map_err(|e| format!("The package is damaged ({e})"))?;
    // The package has one top folder (AgentGo-<version>-win64/); drop it.
    // Compared by path component: packages made by Windows PowerShell's
    // Compress-Archive (0.1.1 as first published) separate with `\`.
    let top: Option<PathBuf> = zip
        .by_index(0)
        .ok()
        .and_then(|f| f.enclosed_name())
        .and_then(|p| p.components().next().map(|c| PathBuf::from(c.as_os_str())));
    let strip = top.filter(|top| {
        (0..zip.len()).all(|i| {
            zip.by_index(i)
                .ok()
                .and_then(|f| f.enclosed_name())
                .is_some_and(|p| p.starts_with(top))
        })
    });
    let total: u64 = (0..zip.len())
        .filter_map(|i| zip.by_index(i).ok().map(|f| f.size()))
        .sum::<u64>()
        .max(1);
    let mut done = 0u64;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
        // enclosed_name refuses absolute paths and `..` (zip slip).
        let Some(name) = entry.enclosed_name() else {
            continue;
        };
        let name = match &strip {
            Some(top) => match name.strip_prefix(top) {
                Ok(rest) => rest.to_path_buf(),
                Err(_) => continue,
            },
            None => name,
        };
        if name.as_os_str().is_empty() {
            continue;
        }
        let path = dir.join(&name);
        if entry.is_dir() {
            std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;
            continue;
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut out = File::create(&path)
            .map_err(|e| format!("{} could not be written ({e})", name.display()))?;
        std::io::copy(&mut entry, &mut out)
            .map_err(|e| format!("{} could not be unpacked ({e})", name.display()))?;
        done += entry.size();
        if i % 8 == 0 {
            log(
                proxy,
                format!("unpack  {}", name.display()).replace('\\', "/"),
            );
            progress(proxy, 43.0 + done as f64 / total as f64 * 40.0);
        }
    }
    Ok(())
}

/// Checks every file against the package's own SHA256SUMS.txt.
fn check_sums(proxy: &Proxy, dir: &Path) -> Result<(), String> {
    let Ok(sums) = std::fs::read_to_string(dir.join("SHA256SUMS.txt")) else {
        return Ok(());
    };
    let lines: Vec<&str> = sums.lines().filter(|l| !l.trim().is_empty()).collect();
    for (i, line) in lines.iter().enumerate() {
        let Some((hash, name)) = line.split_once("  ") else {
            continue;
        };
        if name == "SHA256SUMS.txt" || name.contains("..") {
            continue;
        }
        let file = File::open(dir.join(name)).map_err(|_| {
            format!("{name} is missing from the package. Download AgentGo Setup again.")
        })?;
        let actual = hash_reader(file, |_| {}).map_err(|e| e.to_string())?;
        if !actual.eq_ignore_ascii_case(hash.trim()) {
            return Err(format!(
                "{name} does not match its checksum. Download AgentGo Setup again."
            ));
        }
        if i % 4 == 0 {
            progress(proxy, 83.0 + i as f64 / lines.len().max(1) as f64 * 5.0);
        }
    }
    log(proxy, format!("{} files match SHA256SUMS.txt", lines.len()));
    Ok(())
}

fn write_settings(chosen: &Value) -> std::io::Result<()> {
    let path = data_root().join("config").join("settings.json");
    let mut settings = std::fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| {
            json!({
                "schema_version": 1, "theme": "dark", "accent": "#FF6A2B", "searchEngine": "Google",
                "motion": true, "hardwareAcceleration": true, "captchaNotifications": true,
                "downloadsEnabled": false, "adBlock": true, "fingerprintProtection": true, "vpn": false,
                "nativeControl": false, "controlMode": "approval", "agentTabs": "auto", "profileId": "default"
            })
        });
    const KEYS: [&str; 13] = [
        "theme",
        "autoUpdate",
        "dangerousSiteProtection",
        "blockThirdPartyCookies",
        "stripTracking",
        "searchEngine",
        "adBlock",
        "fingerprintProtection",
        "controlMode",
        "downloadsEnabled",
        "motion",
        "siteDark",
        "hardwareAcceleration",
    ];
    if let (Some(target), Some(source)) = (settings.as_object_mut(), chosen.as_object()) {
        for key in KEYS {
            if let Some(value) = source.get(key) {
                target.insert(key.to_string(), value.clone());
            }
        }
    }
    std::fs::create_dir_all(path.parent().unwrap_or(Path::new(".")))?;
    let partial = path.with_extension("json.tmp");
    std::fs::write(&partial, serde_json::to_vec_pretty(&settings)?)?;
    std::fs::rename(&partial, &path)
}

fn desktop() -> PathBuf {
    hidden("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "[Environment]::GetFolderPath('Desktop')",
        ])
        .output()
        .ok()
        .map(|o| PathBuf::from(String::from_utf8_lossy(&o.stdout).trim()))
        .filter(|p| p.is_dir())
        .unwrap_or_else(|| env_path("USERPROFILE").join("Desktop"))
}

fn shortcut(link: &Path, target: &Path, dir: &Path) -> Result<(), String> {
    if let Some(parent) = link.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    // Paths travel in environment variables, never inside the script text.
    let script = "$s=(New-Object -ComObject WScript.Shell).CreateShortcut($env:AG_LNK);$s.TargetPath=$env:AG_TARGET;$s.WorkingDirectory=$env:AG_DIR;$s.IconLocation=$env:AG_TARGET+',0';$s.Description='AgentGo Browser';$s.Save()";
    let ok = hidden("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            script,
        ])
        .env("AG_LNK", link)
        .env("AG_TARGET", target)
        .env("AG_DIR", dir)
        .status()
        .is_ok_and(|s| s.success());
    if ok {
        Ok(())
    } else {
        Err(format!(
            "The shortcut {} could not be created",
            link.display()
        ))
    }
}

fn set_user_path(dir: &Path, add: bool) {
    let Ok(env) = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags("Environment", KEY_READ | KEY_WRITE)
    else {
        return;
    };
    let (current, kind) = match env.get_raw_value("Path") {
        Ok(raw) => {
            let text: String = env.get_value("Path").unwrap_or_default();
            (text, raw.vtype)
        }
        Err(_) => (String::new(), REG_EXPAND_SZ),
    };
    let me = dir
        .to_string_lossy()
        .trim_end_matches('\\')
        .to_ascii_lowercase();
    let mut parts: Vec<String> = current
        .split(';')
        .filter(|p| !p.is_empty() && p.trim_end_matches('\\').to_ascii_lowercase() != me)
        .map(str::to_string)
        .collect();
    if add {
        parts.push(dir.to_string_lossy().into_owned());
    }
    let joined = parts.join(";");
    if joined == current {
        return;
    }
    let mut bytes: Vec<u8> = joined
        .encode_utf16()
        .chain([0])
        .flat_map(u16::to_le_bytes)
        .collect();
    if bytes.is_empty() {
        bytes = vec![0, 0];
    }
    let vtype = if kind == REG_SZ {
        REG_SZ
    } else {
        REG_EXPAND_SZ
    };
    let _ = env.set_raw_value("Path", &RegValue { bytes, vtype });
    broadcast_environment();
}

fn broadcast_environment() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SendMessageTimeoutW, HWND_BROADCAST, SMTO_ABORTIFHUNG, WM_SETTINGCHANGE,
    };
    let name: Vec<u16> = "Environment".encode_utf16().chain([0]).collect();
    unsafe {
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            0,
            name.as_ptr() as isize,
            SMTO_ABORTIFHUNG,
            3000,
            std::ptr::null_mut(),
        );
    }
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

/// Copies this program, without any appended package, as the uninstaller.
fn copy_stub(to: &Path) -> std::io::Result<()> {
    let me = std::env::current_exe()?;
    if me == to {
        return Ok(());
    }
    let keep = match appended() {
        Some((offset, _, _)) => offset,
        None => std::fs::metadata(&me)?.len(),
    };
    let mut input = File::open(&me)?.take(keep);
    let mut output = File::create(to)?;
    std::io::copy(&mut input, &mut output)?;
    Ok(())
}

fn dir_size_kb(dir: &Path) -> u64 {
    fn walk(dir: &Path) -> u64 {
        std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| match e.file_type() {
                Ok(t) if t.is_dir() => walk(&e.path()),
                _ => e.metadata().map(|m| m.len()).unwrap_or(0),
            })
            .sum()
    }
    walk(dir) / 1024
}

fn register(dir: &Path, setup: &Path, version: &str) -> std::io::Result<()> {
    let (key, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(UNINSTALL_KEY)?;
    let icon = dir.join("agentgo.exe");
    key.set_value("DisplayName", &"AgentGo")?;
    key.set_value("DisplayVersion", &version.to_string())?;
    key.set_value("Publisher", &"AgentGo")?;
    key.set_value("DisplayIcon", &icon.to_string_lossy().into_owned())?;
    key.set_value("InstallLocation", &dir.to_string_lossy().into_owned())?;
    key.set_value(
        "UninstallString",
        &format!("\"{}\" --uninstall", setup.display()),
    )?;
    key.set_value("URLInfoAbout", &format!("https://github.com/{REPO}"))?;
    key.set_value("NoModify", &1u32)?;
    key.set_value("NoRepair", &1u32)?;
    key.set_value("EstimatedSize", &(dir_size_kb(dir) as u32))?;
    Ok(())
}

/* ------------------------------------------------------------------ */
/* Uninstall                                                          */
/* ------------------------------------------------------------------ */

fn uninstall(proxy: &Proxy, remove_profile: bool) -> Result<String, String> {
    let dir = registered().map(|(d, _)| d).unwrap_or_else(current_dir);
    stage(proxy, "u_stop", "run");
    if agentgo_running() {
        return Err("AgentGo is running. Close it, then try again.".into());
    }
    progress(proxy, 10.0);
    stage(proxy, "u_stop", "ok");

    stage(proxy, "u_agents", "run");
    for (host, args) in [
        (
            "claude",
            vec!["mcp", "remove", "--scope", "user", "agentgo"],
        ),
        ("claude", vec!["mcp", "remove", "agentgo"]),
        ("codex", vec!["mcp", "remove", "agentgo"]),
        ("droid", vec!["mcp", "remove", "agentgo"]),
    ] {
        if host_found(host) {
            let _ = hidden("cmd").arg("/C").arg(host).args(&args).output();
        }
    }
    let skill = env_path("USERPROFILE").join(r".claude\skills\agentgo");
    if skill.exists() {
        let _ = std::fs::remove_dir_all(&skill);
    }
    log(proxy, "Agent registrations removed");
    progress(proxy, 30.0);
    stage(proxy, "u_agents", "ok");

    stage(proxy, "u_files", "run");
    let _ = std::fs::remove_file(
        env_path("APPDATA").join(r"Microsoft\Windows\Start Menu\Programs\AgentGo.lnk"),
    );
    let _ = std::fs::remove_file(desktop().join("AgentGo.lnk"));
    if (dir.join("agentgo.exe").exists() || dir.join("AgentGo-Setup.exe").exists())
        && !touches_user_data(&dir)
    {
        std::fs::remove_dir_all(&dir).or_else(|e| {
            // A silent uninstall's caller keeps running AgentGo-Setup.exe from
            // this folder while it waits for the result: once that is the only
            // file left, remove the folder shortly after the caller exits.
            let rest: Vec<_> = std::fs::read_dir(&dir)
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.file_name())
                .collect();
            if rest.len() == 1 && rest[0].eq_ignore_ascii_case("AgentGo-Setup.exe") {
                let _ = hidden("cmd")
                    .arg("/C")
                    .raw_arg(format!(
                        "ping 127.0.0.1 -n 4 >nul & rmdir /s /q \"{}\"",
                        dir.display()
                    ))
                    .current_dir(std::env::temp_dir())
                    .creation_flags(NO_WINDOW | DETACHED)
                    .spawn();
                Ok(())
            } else {
                Err(format!(
                    "{} could not be removed ({e}). Close any window using it and try again.",
                    dir.display()
                ))
            }
        })?;
    }
    log(proxy, format!("Removed {}", dir.display()));
    progress(proxy, 65.0);
    stage(proxy, "u_files", "ok");

    if remove_profile {
        stage(proxy, "u_profile", "run");
        let root = data_root();
        if root.exists() {
            std::fs::remove_dir_all(&root)
                .map_err(|e| format!("Your profile could not be removed ({e})"))?;
        }
        progress(proxy, 85.0);
        stage(proxy, "u_profile", "ok");
    }

    stage(proxy, "u_windows", "run");
    set_user_path(&dir, false);
    let _ = RegKey::predef(HKEY_CURRENT_USER).delete_subkey_all(UNINSTALL_KEY);
    progress(proxy, 100.0);
    stage(proxy, "u_windows", "ok");
    Ok(if remove_profile {
        "AgentGo and your profile are gone.".into()
    } else {
        format!(
            "Your profile is still in {}, in case you come back.",
            data_root().display()
        )
    })
}

/// An uninstaller cannot delete the folder it runs from: run from a copy in
/// the temp folder instead, which removes itself afterwards.
fn relaunch_from_temp_if_needed() {
    let args: Vec<String> = std::env::args().collect();
    let moves_install = args.iter().any(|a| a == "--uninstall" || a == "--update");
    if !moves_install || args.iter().any(|a| a == "--from-temp") {
        return;
    }
    let Ok(me) = std::env::current_exe() else {
        return;
    };
    let copy = std::env::temp_dir().join(format!("agentgo-uninstall-{}.exe", std::process::id()));
    if std::fs::copy(&me, &copy).is_err() {
        return;
    }
    let mut command = Command::new(&copy);
    command.args(args.iter().skip(1)).arg("--from-temp");
    if args.iter().any(|a| a == "--silent") {
        // A silent uninstall reports its result: wait for the copy.
        if let Ok(status) = command.status() {
            std::process::exit(status.code().unwrap_or(1));
        }
    } else if command.creation_flags(DETACHED).spawn().is_ok() {
        std::process::exit(0);
    }
}

/// `--silent`: install or uninstall without a window, for scripted
/// deployments. Options: `--dir <path>`, `--no-shortcuts`, `--no-path`,
/// `--no-agents`, and for uninstall `--remove-profile`. Progress goes to
/// `%TEMP%\agentgo-setup.log`; the exit code is 0 on success.
fn run_silent(uninstall_mode: bool) -> ! {
    let args: Vec<String> = std::env::args().collect();
    let has = |flag: &str| args.iter().any(|a| a == flag);
    let value = |flag: &str| {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let (remove_profile, no_shortcuts, no_path, no_agents) = (
        has("--remove-profile"),
        has("--no-shortcuts"),
        has("--no-path"),
        has("--no-agents"),
    );
    let chosen_dir = value("--dir");
    let log_path = std::env::temp_dir().join("agentgo-setup.log");
    let event_loop = EventLoopBuilder::<Ui>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    let worker = proxy.clone();
    std::thread::spawn(move || {
        let result = if uninstall_mode {
            uninstall(&worker, remove_profile)
        } else {
            let dir = chosen_dir
                .map(PathBuf::from)
                .or_else(|| registered().map(|(d, _)| d))
                .unwrap_or_else(default_dir);
            let agents = !no_agents;
            let options = json!({
                "dir": dir.to_string_lossy(),
                "startMenu": !no_shortcuts,
                "desktop": !no_shortcuts,
                "path": !no_path,
                "claude": agents && host_found("claude"),
                "codex": agents && host_found("codex"),
                "droid": agents && host_found("droid"),
                "skill": agents && env_path("USERPROFILE").join(".claude").is_dir(),
                // Privacy and security stay at their defaults: all on.
                "settings": {},
            });
            install(&worker, &options)
        };
        let code = match result {
            Ok(note) => {
                emit(&worker, json!({"type": "done", "note": note}));
                0
            }
            Err(message) => {
                emit(&worker, json!({"type": "error", "message": message}));
                1
            }
        };
        let _ = worker.send_event(Ui::Exit(code));
    });
    let mut log = std::fs::File::create(&log_path).ok();
    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::UserEvent(Ui::Eval(script)) => {
                if let Some(log) = log.as_mut() {
                    let line = script
                        .trim_start_matches("window.__agentgo&&window.__agentgo(")
                        .trim_end_matches(')');
                    let _ = writeln!(log, "{line}");
                }
            }
            Event::UserEvent(Ui::Exit(code)) => {
                delete_self_later();
                *control_flow = ControlFlow::ExitWithCode(code);
            }
            _ => {}
        }
    })
}

/// What `--update` was asked to do (passed by the browser).
struct UpdateArgs {
    install: PathBuf,
    staged: PathBuf,
    backup: PathBuf,
    wait_pid: u32,
    relaunch: bool,
}

fn update_args() -> Option<UpdateArgs> {
    let args: Vec<String> = std::env::args().collect();
    let value = |flag: &str| {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    Some(UpdateArgs {
        install: PathBuf::from(value("--install")?),
        staged: PathBuf::from(value("--staged")?),
        backup: PathBuf::from(value("--backup")?),
        wait_pid: value("--wait-pid")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0),
        relaunch: args.iter().any(|a| a == "--relaunch"),
    })
}

fn wait_for_exit(pid: u32, timeout_ms: u32) {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE,
    };
    if pid == 0 {
        return;
    }
    // SAFETY: a handle to another process opened for waiting only, closed after.
    unsafe {
        let handle = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
        if !handle.is_null() {
            WaitForSingleObject(handle, timeout_ms);
            CloseHandle(handle);
        }
    }
}

/// Swaps a staged, verified version into the install folder, keeping the
/// current one for rollback (what update.ps1 does, with the film shown).
fn apply_update(proxy: &Proxy, job: &UpdateArgs) -> Result<(String, bool), String> {
    if touches_user_data(&job.install) || touches_user_data(&job.staged) {
        return Err("The update was asked to replace a folder that holds your profile; nothing was changed.".into());
    }
    stage(proxy, "w_wait", "run");
    wait_for_exit(job.wait_pid, 120_000);
    std::thread::sleep(std::time::Duration::from_millis(400));
    if agentgo_running() {
        std::thread::sleep(std::time::Duration::from_secs(3));
    }
    progress(proxy, 15.0);
    stage(proxy, "w_wait", "ok");

    stage(proxy, "w_check", "run");
    if !job.staged.join("agentgo.exe").is_file() {
        return Err(format!(
            "The new version is missing from {}.",
            job.staged.display()
        ));
    }
    check_sums(proxy, &job.staged)?;
    let version = std::fs::read(job.staged.join("BUILD-INFO.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| v["version"].as_str().map(str::to_string))
        .unwrap_or_default();
    progress(proxy, 40.0);
    stage(proxy, "w_check", "ok");

    stage(proxy, "w_keep", "run");
    let _ = std::fs::remove_dir_all(&job.backup);
    std::fs::create_dir_all(&job.backup).map_err(|e| e.to_string())?;
    let entries: Vec<_> = std::fs::read_dir(&job.install)
        .map_err(|e| e.to_string())?
        .flatten()
        .filter(|e| e.file_name() != "updates")
        .collect();
    for entry in &entries {
        std::fs::rename(entry.path(), job.backup.join(entry.file_name())).map_err(|e| {
            format!(
                "{} is in use ({e}). Close AgentGo and try again.",
                entry.file_name().to_string_lossy()
            )
        })?;
    }
    progress(proxy, 60.0);
    stage(proxy, "w_keep", "ok");

    stage(proxy, "w_place", "run");
    let placed = (|| -> std::io::Result<()> {
        for entry in std::fs::read_dir(&job.staged)?.flatten() {
            let to = job.install.join(entry.file_name());
            if entry.file_type()?.is_dir() {
                copy_dir(&entry.path(), &to)?;
            } else {
                std::fs::copy(entry.path(), &to)?;
            }
            log(
                proxy,
                format!("place  {}", entry.file_name().to_string_lossy()),
            );
        }
        Ok(())
    })();
    if let Err(error) = placed {
        // A half copy is worse than the old version: put it back.
        for entry in std::fs::read_dir(&job.install)
            .into_iter()
            .flatten()
            .flatten()
        {
            if entry.file_name() != "updates" {
                let _ = std::fs::remove_dir_all(entry.path());
                let _ = std::fs::remove_file(entry.path());
            }
        }
        for entry in std::fs::read_dir(&job.backup)
            .into_iter()
            .flatten()
            .flatten()
        {
            let _ = std::fs::rename(entry.path(), job.install.join(entry.file_name()));
        }
        return Err(format!(
            "The new version could not be put in place ({error}); the previous one is back."
        ));
    }
    // Setup's own uninstaller is not in a release package: keep it.
    let setup = job.install.join("AgentGo-Setup.exe");
    if !setup.exists() {
        let _ = std::fs::copy(job.backup.join("AgentGo-Setup.exe"), &setup);
    }
    let updates = job.install.join("updates");
    let _ = std::fs::remove_file(updates.join("pending.json"));
    let _ = std::fs::write(
        updates.join("applied.json"),
        json!({"from": job.staged.display().to_string(), "backup": job.backup.display().to_string(), "version": version}).to_string(),
    );
    if registered().is_some_and(|(dir, _)| dir == job.install) {
        let _ = register(&job.install, &setup, &version);
    }
    progress(proxy, 90.0);
    stage(proxy, "w_place", "ok");

    stage(proxy, "w_start", "run");
    let mut relaunched = false;
    if job.relaunch {
        let launcher = if job.install.join("agentgo-browser.exe").exists() {
            job.install.join("agentgo-browser.exe")
        } else {
            job.install.join("agentgo.exe")
        };
        relaunched = Command::new(&launcher)
            .arg("run")
            .arg("--restore")
            .current_dir(&job.install)
            .creation_flags(DETACHED)
            .spawn()
            .is_ok();
    }
    progress(proxy, 100.0);
    stage(proxy, "w_start", if job.relaunch { "ok" } else { "skip" });
    Ok((version, relaunched))
}

fn delete_self_later() {
    if !std::env::args().any(|a| a == "--from-temp") {
        return;
    }
    if let Ok(me) = std::env::current_exe() {
        let _ = hidden("cmd")
            .arg("/C")
            // Verbatim: cmd.exe does not understand the \" escaping `arg` adds.
            .raw_arg(format!(
                "ping 127.0.0.1 -n 3 >nul & del /f /q \"{}\"",
                me.display()
            ))
            .creation_flags(NO_WINDOW | DETACHED)
            .spawn();
    }
}

/* ------------------------------------------------------------------ */
/* Import (through the installed agentgo-tool)                        */
/* ------------------------------------------------------------------ */

fn tool_json(args: &[&str]) -> Value {
    let tool = current_dir().join("agentgo-tool.exe");
    match hidden(&tool).arg("import").args(args).output() {
        Ok(out) => serde_json::from_slice(&out.stdout)
            .unwrap_or_else(|_| json!({"error": "the importer gave no answer"})),
        Err(e) => json!({"error": format!("the importer could not start ({e})")}),
    }
}

/* ------------------------------------------------------------------ */
/* Window                                                             */
/* ------------------------------------------------------------------ */

fn spawn(work: impl FnOnce() + Send + 'static) {
    std::thread::spawn(work);
}

fn handle(proxy: &Proxy, message: &str, uninstall_mode: bool) {
    let Ok(msg) = serde_json::from_str::<Value>(message) else {
        return;
    };
    let p = proxy.clone();
    match msg["cmd"].as_str().unwrap_or("") {
        "drag" => {
            let _ = proxy.send_event(Ui::Drag);
        }
        "minimize" => {
            let _ = proxy.send_event(Ui::Minimize);
        }
        "close" => {
            let _ = proxy.send_event(Ui::Close);
        }
        "browse" => {
            let _ = proxy.send_event(Ui::Browse(msg["dir"].as_str().unwrap_or("").to_string()));
        }
        "import_csv" => {
            let _ = proxy.send_event(Ui::PickCsv);
        }
        "init" => spawn(move || {
            let existing = registered();
            let dir = existing
                .as_ref()
                .map(|(d, _)| d.clone())
                .unwrap_or_else(default_dir);
            if let Ok(mut slot) = INSTALL_DIR.lock() {
                *slot = Some(dir.clone());
            }
            let settings = std::fs::read(data_root().join(r"config\settings.json"))
                .ok()
                .and_then(|b| serde_json::from_slice::<Value>(&b).ok());
            let version = if let Some(job) = update_args() {
                std::fs::read(job.staged.join("BUILD-INFO.json"))
                    .ok()
                    .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                    .and_then(|v| v["version"].as_str().map(str::to_string))
                    .unwrap_or_default()
            } else if uninstall_mode {
                existing
                    .as_ref()
                    .map(|(_, v)| v.clone())
                    .unwrap_or_default()
            } else {
                appended_version().unwrap_or_default()
            };
            emit(
                &p,
                json!({
                    "type": "info",
                    "dir": dir.to_string_lossy(),
                    "version": version,
                    "existing": existing.is_some(),
                    "existingVersion": existing.map(|(_, v)| v),
                    "settings": settings,
                    "hosts": {
                        "claude": host_found("claude"),
                        "codex": host_found("codex"),
                        "droid": host_found("droid"),
                        "claudeDir": env_path("USERPROFILE").join(".claude").is_dir(),
                    },
                }),
            );
        }),
        "install" => {
            let options = msg["options"].clone();
            spawn(move || match install(&p, &options) {
                Ok(note) => emit(&p, json!({"type": "done", "note": note})),
                Err(message) => emit(&p, json!({"type": "error", "message": message})),
            });
        }
        "update" => {
            spawn(move || match update_args() {
                Some(job) => match apply_update(&p, &job) {
                    Ok((version, relaunched)) => emit(
                        &p,
                        json!({
                            "type": "done", "version": version, "relaunched": relaunched,
                            "note": if relaunched { "AgentGo is opening on the new version." } else { "The new version opens the next time you start AgentGo." },
                        }),
                    ),
                    Err(message) => emit(&p, json!({"type": "error", "message": message})),
                },
                None => emit(
                    &p,
                    json!({"type": "error", "message": "AgentGo did not say what to update."}),
                ),
            });
        }
        "uninstall" => {
            let remove_profile = msg["removeProfile"] == true;
            spawn(move || match uninstall(&p, remove_profile) {
                Ok(note) => emit(&p, json!({"type": "done", "note": note})),
                Err(message) => emit(&p, json!({"type": "error", "message": message})),
            });
        }
        "import_scan" => spawn(move || {
            let mut out = tool_json(&[]);
            out["type"] = json!("import_scan");
            emit(&p, out);
        }),
        "import_run" => {
            let browser = msg["browser"].as_str().unwrap_or("").to_string();
            let id = msg["id"].as_str().unwrap_or("Default").to_string();
            let what = msg["what"].as_str().unwrap_or("").to_string();
            spawn(move || {
                let mut out = tool_json(&["--browser", &browser, "--id", &id, "--what", &what]);
                out["type"] = json!("import_result");
                emit(&p, out);
            });
        }
        "open_export" => {
            let browser = msg["browser"].as_str().unwrap_or("").to_string();
            spawn(move || {
                tool_json(&["--open-export", &browser]);
            });
        }
        "launch_and_close" => {
            let dir = current_dir();
            let launcher = if dir.join("agentgo-browser.exe").exists() {
                dir.join("agentgo-browser.exe")
            } else {
                dir.join("agentgo.exe")
            };
            let _ = Command::new(&launcher)
                .current_dir(&dir)
                .creation_flags(DETACHED)
                .spawn();
            let _ = proxy.send_event(Ui::Close);
        }
        _ => {}
    }
}

fn main() {
    relaunch_from_temp_if_needed();
    let uninstall_mode = std::env::args().any(|a| a == "--uninstall");
    let update_mode = std::env::args().any(|a| a == "--update");
    // An update waits, without a window, until AgentGo has closed; only then
    // does the film start (an automatic update may wait for hours).
    if update_mode {
        if let Some(job) = update_args() {
            wait_for_exit(job.wait_pid, u32::MAX);
        }
    }
    if std::env::args().any(|a| a == "--silent") {
        run_silent(uninstall_mode);
    }

    let event_loop = EventLoopBuilder::<Ui>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    let icon: Option<Icon> = Icon::from_resource(1, None).ok();
    let window = match WindowBuilder::new()
        .with_title(if uninstall_mode {
            "Uninstall AgentGo"
        } else if update_mode {
            "Updating AgentGo"
        } else {
            "AgentGo Setup"
        })
        .with_inner_size(LogicalSize::new(640.0, 700.0))
        .with_resizable(false)
        .with_decorations(false)
        .with_transparent(true)
        .with_undecorated_shadow(false)
        .with_window_icon(icon.clone())
        .with_taskbar_icon(icon)
        .build(&event_loop)
    {
        Ok(window) => window,
        Err(error) => {
            rfd::MessageDialog::new()
                .set_title("AgentGo Setup")
                .set_description(format!("The window could not open: {error}"))
                .show();
            return;
        }
    };

    // Open in the middle of the screen the installer starts on.
    if let Some(monitor) = window
        .current_monitor()
        .or_else(|| window.primary_monitor())
    {
        let (area, origin, size) = (monitor.size(), monitor.position(), window.outer_size());
        window.set_outer_position(tao::dpi::PhysicalPosition::new(
            origin.x + (area.width as i32 - size.width as i32) / 2,
            origin.y + (area.height as i32 - size.height as i32) / 2,
        ));
    }

    let html = UI.replace(
        "'__MODE__'",
        if uninstall_mode {
            "'uninstall'"
        } else if update_mode {
            "'update'"
        } else {
            "'install'"
        },
    );
    let ipc_proxy = proxy.clone();
    let webview = match WebViewBuilder::new()
        .with_html(html)
        .with_transparent(true)
        .with_background_color((0, 0, 0, 0))
        .with_devtools(cfg!(debug_assertions))
        .with_ipc_handler(move |request: wry::http::Request<String>| {
            handle(&ipc_proxy, request.body(), uninstall_mode)
        })
        .build(&window)
    {
        Ok(webview) => webview,
        Err(error) => {
            rfd::MessageDialog::new()
                .set_title("AgentGo Setup")
                .set_description(format!("Microsoft Edge WebView2 is needed to show this installer ({error}). It ships with Windows 11; install it from microsoft.com/edge/webview2 and run Setup again."))
                .show();
            return;
        }
    };

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => *control_flow = ControlFlow::Exit,
            Event::UserEvent(Ui::Eval(script)) => {
                let _ = webview.evaluate_script(&script);
            }
            Event::UserEvent(Ui::Drag) => {
                let _ = window.drag_window();
            }
            Event::UserEvent(Ui::Minimize) => window.set_minimized(true),
            Event::UserEvent(Ui::Close) => {
                delete_self_later();
                *control_flow = ControlFlow::Exit;
            }
            Event::UserEvent(Ui::Browse(start)) => {
                let mut dialog = rfd::FileDialog::new()
                    .set_title("Install AgentGo in…")
                    .set_parent(&window);
                if let Some(parent) = Path::new(&start).parent().filter(|p| p.is_dir()) {
                    dialog = dialog.set_directory(parent);
                }
                if let Some(mut picked) = dialog.pick_folder() {
                    if !picked
                        .file_name()
                        .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case("AgentGo"))
                    {
                        picked.push("AgentGo");
                    }
                    if let Ok(mut slot) = INSTALL_DIR.lock() {
                        *slot = Some(picked.clone());
                    }
                    emit(
                        &proxy,
                        json!({"type": "dir", "dir": picked.to_string_lossy()}),
                    );
                }
            }
            Event::UserEvent(Ui::PickCsv) => {
                let picked = rfd::FileDialog::new()
                    .set_title("Choose the password export (CSV)")
                    .add_filter("Password export", &["csv"])
                    .set_parent(&window)
                    .pick_file();
                let p = proxy.clone();
                match picked {
                    Some(file) => spawn(move || {
                        let path = file.to_string_lossy().into_owned();
                        let mut out = tool_json(&["--csv", &path]);
                        out["type"] = json!("import_result");
                        out["csv"] = json!(true);
                        emit(&p, out);
                    }),
                    None => emit(&p, json!({"type": "import_result", "cancelled": true})),
                }
            }
            _ => {}
        }
    });
}
