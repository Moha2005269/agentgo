//! Download safety: every finished download gets Windows' Mark of the Web and
//! a Microsoft Defender scan. A detected threat blocks opening the file.

use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const SCAN_TIMEOUT: Duration = Duration::from_secs(120);

/// Marks the file as coming from the internet unless it already is.
pub fn mark_of_the_web(file: &Path, source: &str, referrer: &str) -> std::io::Result<bool> {
    let stream = format!("{}:Zone.Identifier", file.display());
    if std::fs::read_to_string(&stream).is_ok() {
        return Ok(false);
    }
    // Only web addresses go in; a blob: or data: URL tells SmartScreen nothing.
    let web = |u: &str| {
        (u.starts_with("http://") || u.starts_with("https://")) && !u.contains(['\r', '\n'])
    };
    let mut text = String::from("[ZoneTransfer]\r\nZoneId=3\r\n");
    if referrer.is_empty() || !web(referrer) {
        text.push_str(&format!(
            "HostUrl={}\r\n",
            if web(source) {
                source
            } else {
                "about:internet"
            }
        ));
    } else {
        text.push_str(&format!(
            "ReferrerUrl={referrer}\r\nHostUrl={}\r\n",
            if web(source) {
                source
            } else {
                "about:internet"
            }
        ));
    }
    std::fs::write(stream, text)?;
    Ok(true)
}

/// Defender's command-line scanner: the newest platform copy, else the
/// in-box one.
fn defender() -> Option<PathBuf> {
    let data = std::env::var_os("ProgramData").map(PathBuf::from)?;
    let platform = data.join("Microsoft/Windows Defender/Platform");
    let newest = std::fs::read_dir(platform)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path().join("MpCmdRun.exe"))
        .filter(|path| path.is_file())
        .max();
    newest.or_else(|| {
        let program = std::env::var_os("ProgramFiles").map(PathBuf::from)?;
        let path = program.join("Windows Defender/MpCmdRun.exe");
        path.is_file().then_some(path)
    })
}

/// Scans one file: `clean`, `threat`, or `unavailable` (no scanner, or it
/// did not answer in time). Never remediates; Defender's own real-time
/// protection keeps its normal behaviour.
pub fn scan_file(file: &Path, source: &str, referrer: &str) -> Value {
    let marked = mark_of_the_web(file, source, referrer).unwrap_or(false);
    let Some(scanner) = defender() else {
        return json!({"status": "unavailable", "scanner": null, "markOfTheWeb": marked,
            "note": "Microsoft Defender's command-line scanner was not found"});
    };
    let child = super::exec::hidden_command(&scanner.to_string_lossy())
        .args(["-Scan", "-ScanType", "3", "-DisableRemediation", "-File"])
        .arg(file)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    let Ok(mut child) = child else {
        return json!({"status": "unavailable", "scanner": "Microsoft Defender", "markOfTheWeb": marked});
    };
    let began = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(code)) => {
                // MpCmdRun: 0 nothing found, 2 a threat was found.
                let status = match code.code() {
                    Some(0) => "clean",
                    Some(2) => "threat",
                    _ => "unavailable",
                };
                return json!({"status": status, "scanner": "Microsoft Defender",
                    "markOfTheWeb": marked, "exitCode": code.code()});
            }
            Ok(None) if began.elapsed() < SCAN_TIMEOUT => {
                std::thread::sleep(Duration::from_millis(100));
            }
            _ => {
                let _ = child.kill();
                return json!({"status": "unavailable", "scanner": "Microsoft Defender",
                    "markOfTheWeb": marked, "note": "the scan did not finish in time"});
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_once_and_scans_a_plain_file() {
        let dir = std::env::temp_dir().join(format!("agentgo-scan-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("plain.txt");
        std::fs::write(&file, "hello").unwrap();
        assert!(
            mark_of_the_web(&file, "https://example.test/f.txt", "https://example.test/").unwrap()
        );
        assert!(
            !mark_of_the_web(&file, "https://example.test/f.txt", "https://example.test/").unwrap()
        );
        let scan = scan_file(&file, "https://example.test/f.txt", "");
        assert!(matches!(
            scan["status"].as_str(),
            Some("clean" | "unavailable")
        ));
        let _ = std::fs::remove_dir_all(dir);
    }
}
