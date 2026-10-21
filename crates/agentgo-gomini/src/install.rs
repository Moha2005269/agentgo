//! Downloading and checking GoMini, the same way from AgentGo Setup and
//! from Settings.
//!
//! Layout under `%LOCALAPPDATA%\AgentGo\gomini` (kept across updates, like
//! the profile):
//!
//! ```text
//! model\GoMini-v6-Q8_0.gguf, LICENSE, NOTICE, THIRD_PARTY_DATA.md
//! runtime\vulkan\llama-server.exe ...   (any GPU)
//! runtime\cpu\llama-server.exe ...      (no usable GPU)
//! installed.json                        what was chosen and checked
//! ```
//!
//! Every file is checked against its pinned SHA-256; the runtime is chosen
//! by asking it which devices it sees, and the result must pass a real
//! decision before it is recorded as installed.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::artifacts::{self, Artifact};
use crate::server::{self, Server};

/// Which llama.cpp build serves the model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Runtime {
    /// GPU through Vulkan.
    Vulkan,
    /// Processor only.
    Cpu,
}

impl Runtime {
    /// `vulkan` or `cpu`.
    pub fn as_str(self) -> &'static str {
        match self {
            Runtime::Vulkan => "vulkan",
            Runtime::Cpu => "cpu",
        }
    }

    fn artifact(self) -> Artifact {
        match self {
            Runtime::Vulkan => artifacts::RUNTIME_VULKAN,
            Runtime::Cpu => artifacts::RUNTIME_CPU,
        }
    }
}

/// Where everything lives.
#[derive(Debug, Clone)]
pub struct Layout {
    /// `%LOCALAPPDATA%\AgentGo\gomini`.
    pub root: PathBuf,
}

impl Layout {
    /// The layout under a root folder.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The default root: `%LOCALAPPDATA%\AgentGo\gomini`.
    pub fn default_root() -> Option<PathBuf> {
        std::env::var_os("LOCALAPPDATA")
            .map(|base| PathBuf::from(base).join("AgentGo").join("gomini"))
    }

    /// The model file.
    pub fn model_path(&self) -> PathBuf {
        self.root.join("model").join(artifacts::MODEL.name)
    }

    /// A runtime's folder.
    pub fn runtime_dir(&self, runtime: Runtime) -> PathBuf {
        self.root.join("runtime").join(runtime.as_str())
    }

    /// A runtime's `llama-server.exe`.
    pub fn server_exe(&self, runtime: Runtime) -> PathBuf {
        server::server_exe_in(&self.runtime_dir(runtime))
    }

    fn record_path(&self) -> PathBuf {
        self.root.join("installed.json")
    }
}

/// A checked installation.
#[derive(Debug, Clone, PartialEq)]
pub struct Installed {
    /// The runtime in use.
    pub runtime: Runtime,
    /// The GPU device id (`Vulkan0`) for the Vulkan runtime.
    pub device: Option<String>,
    /// The GPU's name as the runtime reports it.
    pub device_name: Option<String>,
    /// Milliseconds the self-test decision took (after warm-up).
    pub decision_ms: u64,
}

impl Installed {
    /// As JSON for status reports.
    pub fn to_json(&self) -> Value {
        json!({
            "runtime": self.runtime.as_str(),
            "device": self.device,
            "deviceName": self.device_name,
            "decisionMs": self.decision_ms,
            "model": artifacts::MODEL_NAME,
            "llamaBuild": artifacts::LLAMA_BUILD,
        })
    }
}

/// Progress of an installation, for a progress bar.
#[derive(Debug, Clone, PartialEq)]
pub struct Progress {
    /// What is happening: `runtime`, `model`, `check`, `done`.
    pub stage: &'static str,
    /// Human text for the stage.
    pub text: String,
    /// Bytes done of everything to download.
    pub done: u64,
    /// Bytes to download in total.
    pub total: u64,
}

/// The installation recorded on disk, if its files are all still there.
pub fn installed(layout: &Layout) -> Option<Installed> {
    let record: Value = serde_json::from_slice(&std::fs::read(layout.record_path()).ok()?).ok()?;
    if record["model"] != artifacts::MODEL.sha256 || record["llamaBuild"] != artifacts::LLAMA_BUILD
    {
        return None;
    }
    let runtime = match record["runtime"].as_str()? {
        "vulkan" => Runtime::Vulkan,
        "cpu" => Runtime::Cpu,
        _ => return None,
    };
    let found = Installed {
        runtime,
        device: record["device"].as_str().map(str::to_string),
        device_name: record["deviceName"].as_str().map(str::to_string),
        decision_ms: record["decisionMs"].as_u64().unwrap_or(0),
    };
    (layout.model_path().is_file() && layout.server_exe(runtime).is_file()).then_some(found)
}

/// Bytes a fresh installation downloads (GPU path; the CPU fallback adds a
/// little when it is needed).
pub fn download_size() -> u64 {
    artifacts::RUNTIME_VULKAN.size
        + artifacts::MODEL.size
        + artifacts::MODEL_NOTICES.iter().map(|a| a.size).sum::<u64>()
}

/// Downloads, checks and records GoMini. `cancel` stops it between and
/// during downloads; partial files are kept so a retry resumes.
pub fn install(
    layout: &Layout,
    cancel: &AtomicBool,
    progress: &dyn Fn(Progress),
) -> Result<Installed, String> {
    std::fs::create_dir_all(&layout.root).map_err(|e| e.to_string())?;
    let mut total = download_size();
    let mut done = 0u64;
    let report = |stage: &'static str, text: String, done: u64, total: u64| {
        progress(Progress {
            stage,
            text,
            done,
            total,
        })
    };

    // 1. The GPU runtime, and whether it sees a real GPU.
    report("runtime", "Downloading the GPU runtime".into(), done, total);
    fetch_runtime(layout, Runtime::Vulkan, cancel, &|n| {
        report(
            "runtime",
            "Downloading the GPU runtime".into(),
            done + n,
            total,
        )
    })?;
    done += artifacts::RUNTIME_VULKAN.size;
    let gpu = server::usable_gpu(&server::list_devices(&layout.server_exe(Runtime::Vulkan)));

    // 2. The model and its notices.
    let model_dir = layout.root.join("model");
    for art in std::iter::once(artifacts::MODEL).chain(artifacts::MODEL_NOTICES) {
        let base = done;
        download(&art, &model_dir.join(art.name), cancel, &|n| {
            report("model", "Downloading GoMini".into(), base + n, total)
        })?;
        done += art.size;
    }

    // 3. A real decision on the chosen device; the processor if the GPU fails it.
    report(
        "check",
        "Checking GoMini on this computer".into(),
        done,
        total,
    );
    let mut attempt = gpu.map(|(id, name)| (Runtime::Vulkan, Some(id), Some(name)));
    let mut last_error = String::new();
    for _ in 0..2 {
        let (runtime, device, name) = match attempt.take() {
            Some(choice) => choice,
            None => {
                if !layout.server_exe(Runtime::Cpu).is_file() {
                    total += artifacts::RUNTIME_CPU.size;
                    let base = done;
                    fetch_runtime(layout, Runtime::Cpu, cancel, &|n| {
                        report(
                            "runtime",
                            "Downloading the processor runtime".into(),
                            base + n,
                            total,
                        )
                    })?;
                    done += artifacts::RUNTIME_CPU.size;
                }
                (Runtime::Cpu, None, None)
            }
        };
        match self_test(layout, runtime, device.as_deref()) {
            Ok(ms) => {
                let found = Installed {
                    runtime,
                    device,
                    device_name: name,
                    decision_ms: ms,
                };
                write_record(layout, &found)?;
                report("done", "GoMini is ready".into(), total, total);
                return Ok(found);
            }
            Err(e) if runtime == Runtime::Vulkan => last_error = e,
            Err(e) => return Err(format!("{e} (after the GPU failed: {last_error})")),
        }
    }
    Err(last_error)
}

fn write_record(layout: &Layout, found: &Installed) -> Result<(), String> {
    let mut record = found.to_json();
    record["model"] = json!(artifacts::MODEL.sha256);
    std::fs::write(layout.record_path(), record.to_string()).map_err(|e| e.to_string())
}

/// Removes everything GoMini downloaded.
pub fn remove(layout: &Layout) -> Result<(), String> {
    if layout.root.exists() {
        std::fs::remove_dir_all(&layout.root)
            .map_err(|e| format!("GoMini could not be removed: {e}"))?;
    }
    Ok(())
}

/// The request every installation must answer correctly before it counts:
/// the packaged example (fill the postal code, despite an injected page quote).
pub const SELF_TEST: &str = include_str!("self_test.json");

fn self_test(layout: &Layout, runtime: Runtime, device: Option<&str>) -> Result<u64, String> {
    let server = Server::start_with(
        &layout.server_exe(runtime),
        &layout.model_path(),
        runtime,
        device,
        &layout.root.join("worker.log"),
    )?;
    let mut best = u64::MAX;
    for _ in 0..3 {
        let started = Instant::now();
        let answer = server.post("/v1/systemone", SELF_TEST, 60)?;
        best = best.min(started.elapsed().as_millis() as u64);
        if answer["answers"]["next"]["choice"] != "r748|entry" {
            return Err(format!(
                "GoMini answered the check wrongly on {} ({})",
                runtime.as_str(),
                answer["answers"]["next"]["choice"]
            ));
        }
    }
    Ok(best)
}

fn fetch_runtime(
    layout: &Layout,
    runtime: Runtime,
    cancel: &AtomicBool,
    progress: &dyn Fn(u64),
) -> Result<(), String> {
    let dir = layout.runtime_dir(runtime);
    if server::server_exe_in(&dir).is_file()
        && std::fs::read_to_string(dir.join(".sha256")).unwrap_or_default()
            == runtime.artifact().sha256
    {
        progress(runtime.artifact().size);
        return Ok(());
    }
    let art = runtime.artifact();
    let zip = layout.root.join("downloads").join(art.name);
    download(&art, &zip, cancel, progress)?;
    let _ = std::fs::remove_dir_all(&dir);
    unzip(&zip, &dir)?;
    std::fs::write(dir.join(".sha256"), art.sha256).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&zip);
    if !server::server_exe_in(&dir).is_file() {
        return Err(format!("{} has no llama-server.exe", art.name));
    }
    Ok(())
}

/// SHA-256 of a file, streamed.
pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

/// Downloads one artifact to `dest` with resume, progress and cancel, and
/// keeps it only if its SHA-256 matches.
pub fn download(
    art: &Artifact,
    dest: &Path,
    cancel: &AtomicBool,
    progress: &dyn Fn(u64),
) -> Result<(), String> {
    if dest.metadata().is_ok_and(|m| m.len() == art.size)
        && sha256_file(dest).is_ok_and(|hash| hash == art.sha256)
    {
        progress(art.size);
        return Ok(());
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let part = dest.with_extension("part");
    if part.metadata().is_ok_and(|m| m.len() > art.size) {
        let _ = std::fs::remove_file(&part);
    }
    let mut last_error = String::new();
    for attempt in 0..3 {
        if cancel.load(Ordering::SeqCst) {
            return Err("cancelled".into());
        }
        let mut child = server::hidden(Path::new("curl.exe"))
            .args([
                "-sSfL",
                "--retry",
                "3",
                "--connect-timeout",
                "20",
                "-C",
                "-",
                "-o",
            ])
            .arg(&part)
            .arg(art.url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("could not start curl.exe: {e}"))?;
        let status = loop {
            if cancel.load(Ordering::SeqCst) {
                let _ = child.kill();
                let _ = child.wait();
                return Err("cancelled".into());
            }
            if let Ok(Some(status)) = child.try_wait() {
                break status;
            }
            progress(part.metadata().map(|m| m.len()).unwrap_or(0).min(art.size));
            std::thread::sleep(Duration::from_millis(200));
        };
        let mut stderr = String::new();
        if let Some(mut pipe) = child.stderr.take() {
            let _ = pipe.read_to_string(&mut stderr);
        }
        // 33: the server cannot resume; 416: already complete. Start over once.
        if !status.success() {
            last_error = format!("download of {} failed: {}", art.name, stderr.trim());
            if attempt == 0 && matches!(status.code(), Some(33) | Some(22)) {
                let _ = std::fs::remove_file(&part);
            }
            continue;
        }
        progress(art.size);
        let hash = sha256_file(&part).map_err(|e| e.to_string())?;
        if hash != art.sha256 {
            let _ = std::fs::remove_file(&part);
            last_error = format!(
                "{} did not match its pinned SHA-256 (got {hash}); it was deleted",
                art.name
            );
            continue;
        }
        std::fs::rename(&part, dest).map_err(|e| e.to_string())?;
        return Ok(());
    }
    Err(last_error)
}

fn unzip(zip_path: &Path, dir: &Path) -> Result<(), String> {
    let file = std::fs::File::open(zip_path).map_err(|e| e.to_string())?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("bad runtime archive: {e}"))?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        // enclosed_name refuses absolute paths and `..`.
        let Some(name) = entry.enclosed_name() else {
            continue;
        };
        let path = dir.join(name);
        if entry.is_dir() {
            std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;
            continue;
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut out = std::fs::File::create(&path).map_err(|e| e.to_string())?;
        std::io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_record_counts_only_when_its_files_and_pins_match() {
        let root = std::env::temp_dir().join(format!("gomini-layout-{}", std::process::id()));
        let layout = Layout::new(&root);
        assert_eq!(installed(&layout), None);
        let found = Installed {
            runtime: Runtime::Cpu,
            device: None,
            device_name: None,
            decision_ms: 400,
        };
        write_record_for_test(&layout, &found);
        // The record alone is not an installation.
        assert_eq!(installed(&layout), None);
        std::fs::create_dir_all(layout.runtime_dir(Runtime::Cpu)).unwrap();
        std::fs::write(layout.server_exe(Runtime::Cpu), b"x").unwrap();
        std::fs::create_dir_all(layout.model_path().parent().unwrap()).unwrap();
        std::fs::write(layout.model_path(), b"x").unwrap();
        assert_eq!(installed(&layout), Some(found));
        // A record written for another model is ignored.
        std::fs::write(
            layout.record_path(),
            json!({"runtime": "cpu", "model": "old", "llamaBuild": artifacts::LLAMA_BUILD})
                .to_string(),
        )
        .unwrap();
        assert_eq!(installed(&layout), None);
        remove(&layout).unwrap();
        assert!(!root.exists());
    }

    fn write_record_for_test(layout: &Layout, found: &Installed) {
        std::fs::create_dir_all(&layout.root).unwrap();
        write_record(layout, found).unwrap();
    }

    // Downloads go through Windows' own curl.exe.
    #[cfg(windows)]
    #[test]
    fn a_file_with_the_wrong_hash_is_never_kept() {
        let dir = std::env::temp_dir().join(format!("gomini-dl-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("src.bin");
        std::fs::write(&src, b"hello").unwrap();
        let url: &'static str = Box::leak(
            format!("file:///{}", src.display().to_string().replace('\\', "/")).into_boxed_str(),
        );
        let good = Artifact {
            name: "a.bin",
            url,
            sha256: "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824",
            size: 5,
        };
        let cancel = AtomicBool::new(false);
        download(&good, &dir.join("a.bin"), &cancel, &|_| {}).unwrap();
        assert_eq!(std::fs::read(dir.join("a.bin")).unwrap(), b"hello");
        let bad = Artifact {
            name: "b.bin",
            sha256: "00",
            ..good
        };
        let err = download(&bad, &dir.join("b.bin"), &cancel, &|_| {}).unwrap_err();
        assert!(err.contains("pinned SHA-256"), "{err}");
        assert!(!dir.join("b.bin").exists() && !dir.join("b.part").exists());
        cancel.store(true, Ordering::SeqCst);
        assert_eq!(
            download(
                &Artifact {
                    name: "c.bin",
                    ..good
                },
                &dir.join("c.bin"),
                &cancel,
                &|_| {}
            )
            .unwrap_err(),
            "cancelled"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
