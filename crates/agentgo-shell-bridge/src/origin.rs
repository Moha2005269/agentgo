//! Shell trust boundary.
//!
//! The native bridge is exposed **only** to the exact `agentgo://shell`
//! origin in its main frame. Every other origin — including look-alike
//! schemes, subdomains that do not exist, https pages, and content
//! surfaces — must be refused. This module is the single predicate every
//! bridge entry point calls before handling a message.

/// The one and only origin allowed to call the native bridge.
pub const SHELL_ORIGIN: &str = "agentgo://shell";

/// The URL the trusted shell browser loads.
pub const SHELL_URL: &str = "agentgo://shell/index.html";

/// Whether a frame URL is the trusted shell (exact origin, any path,
/// no credentials, no query tricks).
pub fn is_trusted_shell_url(url: &str) -> bool {
    let Some((scheme, rest)) = url.split_once("://") else {
        return false;
    };
    if scheme != "agentgo" {
        return false;
    }
    // Host must be exactly "shell"; no userinfo, no port.
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    if authority != "shell" {
        return false;
    }
    // Reject embedded credentials or @ in the remainder before the path.
    let path_and_after = &rest[authority_end..];
    if let Some(at) = path_and_after.find('@') {
        // An @ before the first '/' would smuggle userinfo; anything else
        // is a path/query character which is fine.
        let before_slash =
            &path_and_after[..path_and_after.find('/').unwrap_or(path_and_after.len())];
        if at < before_slash.len() {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_shell_origin_accepted() {
        assert!(is_trusted_shell_url("agentgo://shell"));
        assert!(is_trusted_shell_url("agentgo://shell/index.html"));
        assert!(is_trusted_shell_url("agentgo://shell/index.html?v=1"));
        assert!(is_trusted_shell_url("agentgo://shell/"));
    }

    #[test]
    fn lookalikes_rejected() {
        assert!(!is_trusted_shell_url("agentgo://shells"));
        assert!(!is_trusted_shell_url("agentgo://shell.evil.example"));
        assert!(!is_trusted_shell_url("agentgo://evil.example/shell"));
        assert!(!is_trusted_shell_url("https://shell"));
        assert!(!is_trusted_shell_url("http://agentgo://shell"));
        assert!(!is_trusted_shell_url("agentgo://user@shell"));
        assert!(!is_trusted_shell_url("agentgo://shell:8080"));
        assert!(!is_trusted_shell_url("AGENTGO://SHELL"));
        assert!(!is_trusted_shell_url(""));
        assert!(!is_trusted_shell_url("about:blank"));
        assert!(!is_trusted_shell_url("data:text/html,<script>"));
    }

    #[test]
    fn nested_at_in_path_is_fine() {
        // mailto-style tricks must not false-positive the userinfo check.
        assert!(is_trusted_shell_url("agentgo://shell/foo@bar"));
    }
}
