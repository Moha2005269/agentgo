//! Declarative scopes: URL scopes and file scopes.
//!
//! Scopes are attached to sessions (Milestone 3) and workflows so actions
//! can only touch what the operator granted. Evaluation is pure and tested;
//! wiring into the live pipeline lands with the session lease system.

use serde::{Deserialize, Serialize};

/// A URL scope granting navigation/interaction within matched origins.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UrlScope {
    /// Allowed origin patterns (`scheme://host[:port]`, host may be a
    /// `*.suffix` wildcard). Empty list means nothing is allowed.
    pub allowed: Vec<String>,
    /// Blocked origin patterns evaluated after `allowed`.
    pub blocked: Vec<String>,
}

impl UrlScope {
    /// A scope that allows nothing.
    pub fn none() -> Self {
        Self {
            allowed: Vec::new(),
            blocked: Vec::new(),
        }
    }

    /// A scope allowing exactly one origin.
    pub fn origin(origin: impl Into<String>) -> Self {
        Self {
            allowed: vec![origin.into()],
            blocked: Vec::new(),
        }
    }

    /// Whether a URL is inside this scope. Parse failures are rejections.
    pub fn permits(&self, url: &str) -> bool {
        let Some(origin) = origin_of(url) else {
            return false;
        };
        if self.blocked.iter().any(|p| origin_matches(p, &origin)) {
            return false;
        }
        self.allowed.iter().any(|p| origin_matches(p, &origin))
    }
}

/// A file-system scope granting access to selected directories.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileScope {
    /// Granted directory prefixes (absolute, normalized, no `..`).
    pub roots: Vec<String>,
}

impl FileScope {
    /// A scope granting nothing.
    pub fn none() -> Self {
        Self { roots: Vec::new() }
    }

    /// Grants a single directory root.
    pub fn dir(dir: impl Into<String>) -> Self {
        Self {
            roots: vec![dir.into()],
        }
    }

    /// Whether a path is inside this scope. Lexical normalization only —
    /// symlink/hardlink defense is applied by the storage layer with
    /// canonicalization before any I/O.
    pub fn permits(&self, path: &str) -> bool {
        let norm = normalize_separators(path);
        if norm.contains("..") {
            return false;
        }
        self.roots
            .iter()
            .any(|root| norm.starts_with(&normalize_separators(root)))
    }
}

fn origin_of(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let host_port = rest.split(['/', '?', '#']).next()?;
    if host_port.is_empty() {
        return None;
    }
    let scheme = url.split_once("://")?.0;
    Some(format!("{scheme}://{host_port}"))
}

fn origin_matches(pattern: &str, origin: &str) -> bool {
    let pattern = pattern.trim_end_matches('/');
    let origin = origin.trim_end_matches('/');
    let (pat_scheme, pat_host) = pattern.split_once("://").unwrap_or(("", pattern));
    let (scheme, host) = origin.split_once("://").unwrap_or(("", origin));
    if pat_scheme != scheme {
        return false;
    }
    if let Some(suffix) = pat_host.strip_prefix("*.") {
        // `*.example.com` matches `example.com` and any subdomain, but
        // never `notexample.com`.
        host == suffix || host.ends_with(&format!(".{suffix}"))
    } else {
        pat_host == host
    }
}

fn normalize_separators(p: &str) -> String {
    p.replace('\\', "/").to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_scope_exact_origin() {
        let s = UrlScope::origin("https://www.google.com");
        assert!(s.permits("https://www.google.com/search?q=x"));
        assert!(s.permits("https://www.google.com"));
        assert!(!s.permits("https://evil.example/https://www.google.com"));
        assert!(!s.permits("https://www.google.com.evil.example/"));
        assert!(!s.permits("http://www.google.com/"));
    }

    #[test]
    fn url_scope_wildcard_suffix() {
        let s = UrlScope {
            allowed: vec!["https://*.example.com".into()],
            blocked: vec![],
        };
        assert!(s.permits("https://a.example.com/x"));
        assert!(s.permits("https://example.com/x"));
        assert!(!s.permits("https://notexample.com/x"));
    }

    #[test]
    fn url_scope_blocklist_wins() {
        let s = UrlScope {
            allowed: vec!["https://*.example.com".into()],
            blocked: vec!["https://ads.example.com".into()],
        };
        assert!(!s.permits("https://ads.example.com/x"));
        assert!(s.permits("https://ok.example.com/x"));
    }

    #[test]
    fn url_scope_rejects_garbage_and_untrusted_schemes() {
        let s = UrlScope {
            allowed: vec!["https://ok.example".into()],
            blocked: vec![],
        };
        assert!(!s.permits("not a url"));
        assert!(!s.permits("file:///C:/Windows"));
        assert!(!s.permits("javascript:alert(1)"));
    }

    #[test]
    fn file_scope_prefix_and_escape() {
        let s = FileScope::dir("C:/Users/Admin/Downloads");
        assert!(s.permits("C:\\Users\\Admin\\Downloads\\file.txt"));
        assert!(!s.permits("C:/Users/Admin/Documents/file.txt"));
        assert!(!s.permits("C:/Users/Admin/Downloads/../Documents/file.txt"));
    }
}
