//! Verification of an extracted CEF binary distribution.
//!
//! `scripts/bootstrap.ps1` downloads the pinned standard distribution,
//! verifies size + SHA-1 (+ SHA-256 once recorded), and restructures it
//! into the layout cef-rs expects (`<cef_version>/windows64`). This
//! module verifies that layout on demand (doctor, CI, spike startup) so
//! builds fail loudly instead of compiling against a broken runtime.

use std::path::Path;

use crate::pins;

/// Failures of distribution verification.
#[derive(Debug, thiserror::Error)]
pub enum DistroError {
    /// A required file or directory is missing.
    #[error("CEF distribution at {root} is missing {missing}")]
    Missing {
        /// Distro root.
        root: String,
        /// What is missing.
        missing: String,
    },
    /// The version reported by the distro does not match the pin.
    #[error("CEF version mismatch: distro reports {found}, pinned {pinned}")]
    VersionMismatch {
        /// Version found in the distro.
        found: String,
        /// Pinned version.
        pinned: String,
    },
}

/// Result of a successful verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DistroReport {
    /// Verified distro root.
    pub root: String,
    /// CEF version read from the distro headers.
    pub cef_version: String,
    /// Chromium version read from the distro headers.
    pub chromium_version: String,
}

/// Files/dirs that must exist in the restructured windows64 layout.
const REQUIRED: &[&str] = &[
    "libcef.dll",
    "icudtl.dat",
    "resources.pak",
    "chrome_100_percent.pak",
    "locales",
    "libcef.lib",
    "include/include_cef_api.h",
    "libcef_dll",
    "cmake",
];

/// Verifies the cef-rs layout (`<dir>` is the `windows64` directory).
/// Reads `include/cef_version.h` to confirm the pinned CEF/Chromium
/// versions.
pub fn verify_distribution(dir: &Path) -> Result<DistroReport, DistroError> {
    let root = dir.display().to_string();
    for rel in REQUIRED {
        if !dir.join(rel).exists() {
            return Err(DistroError::Missing {
                root,
                missing: (*rel).to_string(),
            });
        }
    }
    let header =
        std::fs::read_to_string(dir.join("include").join("cef_version.h")).map_err(|_| {
            DistroError::Missing {
                root: root.clone(),
                missing: "include/cef_version.h".into(),
            }
        })?;
    let cef_version = header_value(&header, "CEF_VERSION").ok_or_else(|| DistroError::Missing {
        root: root.clone(),
        missing: "CEF_VERSION macro".into(),
    })?;
    let chromium_version =
        header_value(&header, "CHROMIUM_VERSION").ok_or_else(|| DistroError::Missing {
            root: root.clone(),
            missing: "CHROMIUM_VERSION macro".into(),
        })?;
    let pinned_chromium = pins::CEF_VERSION
        .split_once("chromium-")
        .map(|(_, c)| c.to_string())
        .unwrap_or_default();
    if chromium_version != pinned_chromium {
        return Err(DistroError::VersionMismatch {
            found: chromium_version,
            pinned: pinned_chromium,
        });
    }
    Ok(DistroReport {
        root,
        cef_version,
        chromium_version,
    })
}

/// Extracts `#define KEY "value"` from a C header.
fn header_value(header: &str, key: &str) -> Option<String> {
    for line in header.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("#define") {
            let rest = rest.trim();
            if let Some(value) = rest.strip_prefix(key) {
                let value = value.trim().trim_matches('"');
                if !value.is_empty() {
                    return Some(value.to_string());
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str, files: &[&str], version_h: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "agentgo-cef-test-{}-{name}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        for rel in files {
            let p = dir.join(rel);
            if let Some(parent) = p.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&p, b"fixture").unwrap();
        }
        let inc = dir.join("include");
        std::fs::create_dir_all(&inc).unwrap();
        std::fs::write(inc.join("cef_version.h"), version_h).unwrap();
        dir
    }

    #[test]
    fn verifies_a_complete_distribution() {
        let dir = fixture(
            "ok",
            REQUIRED,
            "#define CEF_VERSION \"152.0.5+gb129680+chromium-152.0.7977.54\"\n#define CHROMIUM_VERSION \"152.0.7977.54\"\n",
        );
        let report = verify_distribution(&dir).unwrap();
        assert_eq!(report.chromium_version, "152.0.7977.54");
        assert!(report.cef_version.starts_with("152.0.5+"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_files_reported_exactly() {
        let dir = fixture("missing", &["libcef.dll"], "");
        match verify_distribution(&dir) {
            Err(DistroError::Missing { missing, .. }) => {
                assert_eq!(missing, "icudtl.dat");
            }
            other => panic!("expected Missing, got {other:?}"),
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn version_mismatch_detected() {
        let dir = fixture(
            "mismatch",
            REQUIRED,
            "#define CEF_VERSION \"151.3.24\"\n#define CHROMIUM_VERSION \"151.0.7922.174\"\n",
        );
        match verify_distribution(&dir) {
            Err(DistroError::VersionMismatch { found, pinned }) => {
                assert_eq!(found, "151.0.7922.174");
                assert_eq!(pinned, "152.0.7977.54");
            }
            other => panic!("expected VersionMismatch, got {other:?}"),
        }
        std::fs::remove_dir_all(&dir).ok();
    }
}
