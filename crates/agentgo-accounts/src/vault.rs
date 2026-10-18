//! Token storage. One sealed file per linked provider.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::Error;

/// The tokens of one linked account.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Tokens {
    /// Bearer token for API calls.
    pub access_token: String,
    /// Long-lived token used to get a new access token.
    pub refresh_token: Option<String>,
    /// Unix time after which the access token is no longer valid.
    pub expires_at: Option<u64>,
    /// Scopes the person granted.
    pub scopes: Vec<String>,
    /// Unix time the account was linked.
    pub linked_at: u64,
    /// The OAuth client the tokens belong to (the person's own application).
    #[serde(default)]
    pub client_id: String,
    /// Its secret, for providers that require one from installed apps.
    #[serde(default)]
    pub client_secret: Option<String>,
}

impl std::fmt::Debug for Tokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Tokens(redacted, expires_at: {:?})", self.expires_at)
    }
}

impl Tokens {
    /// Whether the access token should be renewed before use (one minute of
    /// slack for clock differences and the call itself).
    pub fn stale(&self, now: u64) -> bool {
        self.expires_at.is_some_and(|expires| now + 60 >= expires)
    }
}

/// Encrypts and decrypts vault files.
pub trait Seal {
    /// Encrypts `plain`.
    fn seal(&self, plain: &[u8]) -> Result<Vec<u8>, Error>;
    /// Decrypts what [`Seal::seal`] produced.
    fn open(&self, sealed: &[u8]) -> Result<Vec<u8>, Error>;
}

/// Windows DPAPI: the key belongs to the signed-in Windows user, so the file
/// is useless on another machine or under another user account.
#[cfg(windows)]
#[derive(Debug, Default, Clone, Copy)]
pub struct Dpapi;

#[cfg(windows)]
#[allow(unsafe_code)]
impl Dpapi {
    fn run(&self, input: &[u8], protect: bool) -> Result<Vec<u8>, Error> {
        use windows_sys::Win32::Foundation::LocalFree;
        use windows_sys::Win32::Security::Cryptography::{
            CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
        };
        // Ties the blob to this purpose: another DPAPI user of the same
        // Windows account cannot be tricked into opening it for us.
        let purpose = b"AgentGo account vault v1";
        let blob = |bytes: &[u8]| CRYPT_INTEGER_BLOB {
            cbData: bytes.len() as u32,
            pbData: bytes.as_ptr() as *mut u8,
        };
        let (data, entropy) = (blob(input), blob(purpose));
        let mut out = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };
        // SAFETY: every blob points at a live buffer of the stated length; the
        // system allocates `out`, which is copied and freed below.
        let ok = unsafe {
            if protect {
                CryptProtectData(
                    &data,
                    std::ptr::null(),
                    &entropy,
                    std::ptr::null(),
                    std::ptr::null(),
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut out,
                )
            } else {
                CryptUnprotectData(
                    &data,
                    std::ptr::null_mut(),
                    &entropy,
                    std::ptr::null(),
                    std::ptr::null(),
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut out,
                )
            }
        };
        if ok == 0 || out.pbData.is_null() {
            return Err(Error::Vault(std::io::Error::last_os_error().to_string()));
        }
        // SAFETY: on success `out` is a system buffer of `cbData` bytes.
        let bytes = unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize) }.to_vec();
        // SAFETY: `out.pbData` was allocated by the system for this call.
        unsafe { LocalFree(out.pbData.cast()) };
        Ok(bytes)
    }
}

#[cfg(windows)]
impl Seal for Dpapi {
    fn seal(&self, plain: &[u8]) -> Result<Vec<u8>, Error> {
        self.run(plain, true)
    }

    fn open(&self, sealed: &[u8]) -> Result<Vec<u8>, Error> {
        self.run(sealed, false)
    }
}

/// Sealed token files in one directory.
pub struct Vault<S: Seal> {
    dir: PathBuf,
    seal: S,
}

impl<S: Seal> Vault<S> {
    /// A vault in `dir` (created on first save).
    pub fn new(dir: impl Into<PathBuf>, seal: S) -> Self {
        Self {
            dir: dir.into(),
            seal,
        }
    }

    fn file(&self, provider: &str) -> Result<PathBuf, Error> {
        // Account ids are lower-case words; nothing else becomes a file name.
        if provider.is_empty()
            || provider.len() > 80
            || !provider
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        {
            return Err(Error::Unknown("provider"));
        }
        Ok(self.dir.join(format!("{provider}.bin")))
    }

    /// Stores the tokens of `provider`, replacing older ones.
    pub fn save(&self, provider: &str, tokens: &Tokens) -> Result<(), Error> {
        let file = self.file(provider)?;
        let plain = serde_json::to_vec(tokens).map_err(|error| Error::Vault(error.to_string()))?;
        let sealed = self.seal.seal(&plain)?;
        std::fs::create_dir_all(&self.dir).map_err(|error| Error::Vault(error.to_string()))?;
        // Write beside the target and rename, so a crash never leaves half a file.
        let partial = file.with_extension("tmp");
        std::fs::write(&partial, sealed)
            .and_then(|()| std::fs::rename(&partial, &file))
            .map_err(|error| Error::Vault(error.to_string()))
    }

    /// The tokens of `provider`, or `None` when it is not linked. A file that
    /// cannot be opened (copied from another user, damaged) is an error.
    pub fn load(&self, provider: &str) -> Result<Option<Tokens>, Error> {
        let sealed = match std::fs::read(self.file(provider)?) {
            Ok(sealed) => sealed,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(Error::Vault(error.to_string())),
        };
        let plain = self.seal.open(&sealed)?;
        serde_json::from_slice(&plain)
            .map(Some)
            .map_err(|_| Error::Vault("the stored tokens are damaged".into()))
    }

    /// Forgets the tokens of `provider`. Returns whether there were any.
    pub fn remove(&self, provider: &str) -> Result<bool, Error> {
        match std::fs::remove_file(self.file(provider)?) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(Error::Vault(error.to_string())),
        }
    }
}

/// A saved sign-in the browser can type into a page for an agent.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Login {
    /// Exact origin (`https://host[:port]`) the sign-in belongs to.
    pub origin: String,
    /// User name or e-mail address.
    pub username: String,
    /// The password. Leaves this crate only to be typed into the page.
    pub password: String,
}

impl std::fmt::Debug for Login {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Login({} as {}, password redacted)",
            self.origin, self.username
        )
    }
}

impl Login {
    /// Checks what the person typed. The origin is reduced to scheme, host
    /// and port; `http` is accepted only for this computer.
    pub fn new(address: &str, username: &str, password: &str) -> Result<Self, Error> {
        let (scheme, rest) = address
            .split_once("://")
            .ok_or(Error::BadSite("the address must start with https://"))?;
        let authority = rest
            .split(['/', '?', '#'])
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        let (host, port) = authority.split_once(':').unwrap_or((&authority, ""));
        let local = matches!(host, "127.0.0.1" | "localhost");
        if !(scheme == "https" || scheme == "http" && local) {
            return Err(Error::BadSite("the address must start with https://"));
        }
        if host.is_empty()
            || !host
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
            || !port.bytes().all(|b| b.is_ascii_digit())
        {
            return Err(Error::BadSite("the address has no valid host"));
        }
        if username.is_empty()
            || username.len() > 256
            || password.is_empty()
            || password.len() > 1024
            || username.chars().any(char::is_control)
        {
            return Err(Error::BadSite("user name and password are required"));
        }
        Ok(Self {
            origin: format!("{scheme}://{authority}"),
            username: username.to_string(),
            password: password.to_string(),
        })
    }
}

/// All saved sign-ins of a profile, in one sealed file.
pub struct LoginStore<S: Seal> {
    file: PathBuf,
    seal: S,
}

impl<S: Seal> LoginStore<S> {
    /// A store kept in `file`.
    pub fn new(file: impl Into<PathBuf>, seal: S) -> Self {
        Self {
            file: file.into(),
            seal,
        }
    }

    /// Every saved sign-in. A missing file is an empty store.
    pub fn load(&self) -> Result<Vec<Login>, Error> {
        let sealed = match std::fs::read(&self.file) {
            Ok(sealed) => sealed,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(Error::Vault(error.to_string())),
        };
        serde_json::from_slice(&self.seal.open(&sealed)?)
            .map_err(|_| Error::Vault("the saved sign-ins are damaged".into()))
    }

    /// Replaces the stored sign-ins.
    pub fn save(&self, logins: &[Login]) -> Result<(), Error> {
        let fail = |error: std::io::Error| Error::Vault(error.to_string());
        let sealed = self
            .seal
            .seal(&serde_json::to_vec(logins).map_err(|error| Error::Vault(error.to_string()))?)?;
        if let Some(dir) = self.file.parent() {
            std::fs::create_dir_all(dir).map_err(fail)?;
        }
        let partial = self.file.with_extension("tmp");
        std::fs::write(&partial, sealed)
            .and_then(|()| std::fs::rename(&partial, &self.file))
            .map_err(fail)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens() -> Tokens {
        Tokens {
            access_token: "access-SECRET".into(),
            refresh_token: Some("refresh-SECRET".into()),
            expires_at: Some(100),
            scopes: vec!["s".into()],
            linked_at: 1,
            client_id: "c".into(),
            client_secret: None,
        }
    }

    #[cfg(windows)]
    fn temp(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("agentgo-vault-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn tokens_know_when_they_are_stale() {
        assert!(!tokens().stale(0));
        assert!(tokens().stale(40));
        assert!(!Tokens {
            expires_at: None,
            ..tokens()
        }
        .stale(u64::MAX - 100));
    }

    #[test]
    fn saved_sign_ins_are_bound_to_an_exact_origin() {
        let login = Login::new("https://Jobs.Example.com:8443/login?next=/", "ada", "pw").unwrap();
        assert_eq!(login.origin, "https://jobs.example.com:8443");
        assert!(Login::new("http://127.0.0.1:9000/x", "ada", "pw").is_ok());
        for (address, user, password) in [
            ("http://jobs.example.com/", "ada", "pw"),
            ("jobs.example.com", "ada", "pw"),
            ("https://a b/", "ada", "pw"),
            ("https://x.example/", "", "pw"),
            ("https://x.example/", "ada", ""),
        ] {
            assert!(Login::new(address, user, password).is_err(), "{address}");
        }
        assert!(!format!("{login:?}").contains("pw"));
    }

    #[cfg(windows)]
    #[test]
    fn saved_sign_ins_are_encrypted_on_disk() {
        let dir = temp("logins");
        let store = LoginStore::new(dir.join("logins.bin"), Dpapi);
        assert_eq!(store.load().unwrap(), Vec::new());
        let login = Login::new("https://x.example/", "ada", "hunter2-SECRET").unwrap();
        store.save(std::slice::from_ref(&login)).unwrap();
        let raw =
            String::from_utf8_lossy(&std::fs::read(dir.join("logins.bin")).unwrap()).to_string();
        assert!(!raw.contains("SECRET") && !raw.contains("ada"));
        assert_eq!(store.load().unwrap(), vec![login]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(windows)]
    #[test]
    fn the_vault_file_is_encrypted_and_round_trips() {
        let dir = temp("dpapi");
        let vault = Vault::new(&dir, Dpapi);
        assert_eq!(vault.load("github").unwrap(), None);
        vault.save("github", &tokens()).unwrap();
        let raw = std::fs::read(dir.join("github.bin")).unwrap();
        let text = String::from_utf8_lossy(&raw);
        assert!(
            !text.contains("SECRET") && !text.contains("access_token"),
            "tokens are stored in the clear"
        );
        assert_eq!(vault.load("github").unwrap(), Some(tokens()));
        // A blob sealed for another purpose, or a damaged one, does not open.
        std::fs::write(dir.join("github.bin"), &raw[..raw.len() - 1]).unwrap();
        assert!(matches!(vault.load("github"), Err(Error::Vault(_))));
        assert!(vault.remove("github").unwrap());
        assert!(!vault.remove("github").unwrap());
        // Provider ids cannot be used to walk the file system.
        assert_eq!(
            vault.save("../x", &tokens()).err(),
            Some(Error::Unknown("provider"))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
