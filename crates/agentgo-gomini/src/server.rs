//! The resident GoMini worker: upstream `llama-server` with the model loaded
//! once, on 127.0.0.1 and a free port. It is tied to AgentGo with a Windows
//! job object, so it can never outlive the browser, even after a crash.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::http;
use crate::install::{Installed, Layout, Runtime};

/// A running worker. Dropping it stops the process.
pub struct Server {
    child: Child,
    port: u16,
    /// The worker's API key: nothing else on this computer (another program,
    /// a web page calling localhost) can use the model without it.
    key: String,
    #[cfg(windows)]
    job: Option<isize>,
}

/// A program that never opens a console window.
pub(crate) fn hidden(program: &Path) -> Command {
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut command = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    command
}

fn free_port() -> Result<u16, String> {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .and_then(|listener| listener.local_addr())
        .map(|address| address.port())
        .map_err(|e| format!("no free local port: {e}"))
}

/// A fresh random API key for one worker run.
fn new_key() -> Result<String, String> {
    let mut bytes = [0u8; 24];
    getrandom::fill(&mut bytes).map_err(|e| format!("no random source: {e}"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// The arguments the model was checked with (LIMITATIONS_AND_AGENTGO_INTEGRATION.md),
/// with the device chosen at install time.
fn arguments(
    model: &Path,
    runtime: Runtime,
    device: Option<&str>,
    port: u16,
    key: &str,
) -> Vec<String> {
    let threads = std::thread::available_parallelism()
        .map(|n| (n.get() / 2).clamp(2, 8))
        .unwrap_or(4)
        .to_string();
    let mut args: Vec<String> = [
        "-m",
        &model.display().to_string(),
        "-c",
        "1024",
        "-b",
        "1024",
        "-ub",
        "1024",
        "-np",
        "1",
        "-t",
        &threads,
        "-tb",
        &threads,
        "--host",
        "127.0.0.1",
        "--port",
        &port.to_string(),
        "--api-key",
        key,
        "--no-webui",
        "--pooling",
        "none",
        "-fit",
        "off",
        "-fa",
        "off",
        "-lm",
        "none",
        "--alias",
        crate::artifacts::MODEL_NAME,
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    match (runtime, device) {
        (Runtime::Vulkan, Some(device)) => {
            args.extend(["-ngl".into(), "99".into(), "--device".into(), device.into()])
        }
        _ => args.extend(["-ngl".into(), "0".into()]),
    }
    args
}

/// The devices a runtime can use, as `llama-server --list-devices` names them
/// (`Vulkan0: AMD Radeon RX 7800 XT (16368 MiB, ...)`).
pub fn list_devices(server_exe: &Path) -> Vec<String> {
    let Ok(out) = hidden(server_exe)
        .arg("--list-devices")
        .stdin(Stdio::null())
        .output()
    else {
        return Vec::new();
    };
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    text.lines()
        .map(str::trim)
        .filter(|line| line.contains(':') && !line.starts_with("Available"))
        .map(str::to_string)
        .collect()
}

/// The first real GPU in a device list: software renderers do not count.
pub fn usable_gpu(devices: &[String]) -> Option<(String, String)> {
    devices.iter().find_map(|line| {
        let (id, name) = line.split_once(':')?;
        let lower = name.to_ascii_lowercase();
        let software = [
            "llvmpipe",
            "swiftshader",
            "basic render",
            "microsoft basic",
            "lavapipe",
        ]
        .iter()
        .any(|s| lower.contains(s));
        (id.starts_with("Vulkan") && !software)
            .then(|| (id.trim().to_string(), name.trim().to_string()))
    })
}

impl Server {
    /// Starts the worker for an installation and waits until it answers.
    pub fn start(layout: &Layout, installed: &Installed) -> Result<Server, String> {
        Self::start_with(
            &layout.server_exe(installed.runtime),
            &layout.model_path(),
            installed.runtime,
            installed.device.as_deref(),
            &layout.root.join("worker.log"),
        )
    }

    /// Starts `exe` with `model` on `runtime`/`device`.
    pub fn start_with(
        exe: &Path,
        model: &Path,
        runtime: Runtime,
        device: Option<&str>,
        log: &Path,
    ) -> Result<Server, String> {
        if !exe.is_file() || !model.is_file() {
            return Err("GoMini is not installed completely; download it again in Settings".into());
        }
        let port = free_port()?;
        let key = new_key()?;
        let log_file = std::fs::File::create(log).map_err(|e| e.to_string())?;
        let child = hidden(exe)
            .args(arguments(model, runtime, device, port, &key))
            .current_dir(exe.parent().unwrap_or(Path::new(".")))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(log_file))
            .spawn()
            .map_err(|e| format!("GoMini worker could not start: {e}"))?;
        let mut server = Server {
            #[cfg(windows)]
            job: tie_to_parent(&child),
            child,
            port,
            key,
        };
        let deadline = Instant::now() + Duration::from_secs(90);
        loop {
            if let Ok(Some(status)) = server.child.try_wait() {
                let tail = std::fs::read_to_string(log).unwrap_or_default();
                let tail: Vec<&str> = tail.lines().rev().take(6).collect();
                return Err(format!(
                    "GoMini worker stopped ({status}): {}",
                    tail.into_iter().rev().collect::<Vec<_>>().join(" | ")
                ));
            }
            if server.healthy() {
                return Ok(server);
            }
            if Instant::now() > deadline {
                return Err("GoMini worker did not become ready in 90 s".into());
            }
            std::thread::sleep(Duration::from_millis(150));
        }
    }

    /// The port on 127.0.0.1.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Whether the process runs and reports ready.
    pub fn healthy(&self) -> bool {
        http::request(
            self.port,
            &self.key,
            "GET",
            "/health",
            None,
            Duration::from_secs(2),
        )
        .ok()
        .and_then(|(code, body)| (code == 200).then_some(body))
        .and_then(|body| serde_json::from_str::<Value>(&body).ok())
        .is_some_and(|v| v["status"] == "ok")
    }

    /// Whether the process is still alive.
    pub fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// Token ids of `text` as the model's own tokenizer reads it
    /// (special tokens parsed, none added).
    pub fn tokenize(&self, text: &str) -> Result<Vec<i64>, String> {
        let body = json!({"content": text, "add_special": false, "parse_special": true});
        let answer = self.post("/tokenize", &body.to_string(), 10)?;
        answer["tokens"]
            .as_array()
            .ok_or("GoMini tokenizer gave no tokens")?
            .iter()
            .map(|t| t.as_i64().ok_or_else(|| "bad token id".to_string()))
            .collect()
    }

    /// Posts a JSON body and returns the JSON answer (errors carry the
    /// server's own message).
    pub fn post(&self, path: &str, body: &str, timeout_s: u64) -> Result<Value, String> {
        let (code, text) = http::request(
            self.port,
            &self.key,
            "POST",
            path,
            Some(body),
            Duration::from_secs(timeout_s),
        )?;
        let value: Value = serde_json::from_str(&text)
            .map_err(|_| format!("GoMini worker answered {code} with non-JSON"))?;
        if code != 200 {
            let message = value["error"]["message"]
                .as_str()
                .or_else(|| value["error"].as_str())
                .unwrap_or("error");
            return Err(format!("GoMini worker {code}: {message}"));
        }
        Ok(value)
    }
}

#[allow(unsafe_code)]
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        #[cfg(windows)]
        if let Some(job) = self.job.take() {
            // SAFETY: the handle came from CreateJobObjectW and is closed once.
            unsafe {
                windows_sys::Win32::Foundation::CloseHandle(job as _);
            }
        }
    }
}

/// Puts the worker in a job that the system ends when AgentGo ends.
#[cfg(windows)]
#[allow(unsafe_code)]
fn tie_to_parent(child: &Child) -> Option<isize> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    // SAFETY: plain Win32 calls on a handle we own; failure leaves the
    // worker running untied, which Drop still stops on a normal exit.
    unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job.is_null() {
            return None;
        }
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let set = SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const _,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );
        if set == 0 || AssignProcessToJobObject(job, child.as_raw_handle() as _) == 0 {
            windows_sys::Win32::Foundation::CloseHandle(job);
            return None;
        }
        Some(job as isize)
    }
}

/// Where a runtime's `llama-server.exe` is.
pub fn server_exe_in(dir: &Path) -> PathBuf {
    dir.join("llama-server.exe")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_a_real_gpu_and_ignores_software_renderers() {
        let devices = vec![
            "Vulkan0: llvmpipe (LLVM 15.0.7, 256 bits) (1024 MiB, 1024 MiB free)".to_string(),
            "Vulkan1: AMD Radeon RX 7800 XT (16368 MiB, 15409 MiB free)".to_string(),
        ];
        assert_eq!(
            usable_gpu(&devices),
            Some((
                "Vulkan1".into(),
                "AMD Radeon RX 7800 XT (16368 MiB, 15409 MiB free)".into()
            ))
        );
        assert_eq!(usable_gpu(&devices[..1]), None);
        assert_eq!(usable_gpu(&[]), None);
    }

    #[test]
    fn gpu_and_cpu_arguments() {
        let model = Path::new("m.gguf");
        let gpu = arguments(model, Runtime::Vulkan, Some("Vulkan0"), 5, "k");
        assert!(gpu.windows(2).any(|w| w == ["--device", "Vulkan0"]));
        assert!(gpu.windows(2).any(|w| w == ["-ngl", "99"]));
        let cpu = arguments(model, Runtime::Cpu, None, 5, "k");
        assert!(cpu.windows(2).any(|w| w == ["-ngl", "0"]));
        assert!(!cpu.iter().any(|a| a == "--device"));
        assert!(cpu.windows(2).any(|w| w == ["--host", "127.0.0.1"]));
        assert!(cpu.windows(2).any(|w| w == ["--api-key", "k"]));
        assert_ne!(new_key().unwrap(), new_key().unwrap());
    }
}
