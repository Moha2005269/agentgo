//! Account Center core.
//!
//! Everything here is pure and testable without a browser: which providers
//! exist, how a sign-in is started and checked (OAuth 2.0 authorization code
//! with PKCE on a loopback redirect), which API addresses a connector may
//! reach, and how tokens are kept (sealed with Windows DPAPI for the current
//! user). The browser runtime adds the windows, the network and the consent
//! screen on top.
//!
//! Rules this crate enforces, because an open-source browser cannot rely on
//! anything being secret:
//! - only the S256 PKCE method, a fresh verifier and `state` per attempt;
//! - the redirect is accepted only on `127.0.0.1:<the port we opened>` at
//!   `/callback`, with exactly one `code` and exactly our `state`;
//! - a connector call can only reach the API host of its connector;
//! - tokens never appear in `Debug` output and leave this crate only sealed
//!   or as an `Authorization` header value for the network layer.

mod vault;

use sha2::{Digest, Sha256};

#[cfg(windows)]
pub use vault::Dpapi;
pub use vault::{Login, LoginStore, Seal, Tokens, Vault};

/// Why a sign-in step or a connector call was refused.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    /// The redirect did not come back to the address we opened.
    #[error("the sign-in answer came to the wrong address")]
    WrongRedirect,
    /// `state` is missing, repeated or not ours: someone else started this.
    #[error("the sign-in answer does not belong to this sign-in attempt")]
    StateMismatch,
    /// The provider reported an error or the person said no.
    #[error("the provider refused: {0}")]
    Denied(String),
    /// No usable authorization code.
    #[error("the sign-in answer carries no valid code")]
    BadCode,
    /// The token endpoint answered with something that is not a bearer token.
    #[error("the provider returned an unusable token response")]
    BadTokenResponse,
    /// A connector call tried to leave its API.
    #[error("connector calls must stay on the connector's API: {0}")]
    BadConnectorCall(&'static str),
    /// Unknown provider, connector, or a provider without that feature.
    #[error("unknown {0}")]
    Unknown(&'static str),
    /// A site the person tried to add is not acceptable.
    #[error("this site cannot be added: {0}")]
    BadSite(&'static str),
    /// The operating system could not seal or open the vault.
    #[error("the account vault is unavailable: {0}")]
    Vault(String),
}

/// OAuth endpoints of a provider. All of them are `https`.
#[derive(Debug, Clone, Copy)]
pub struct OAuth {
    /// Authorization endpoint the person is sent to.
    pub authorize: &'static str,
    /// Token endpoint (code exchange and refresh).
    pub token: &'static str,
    /// Revocation endpoint, when the provider has one.
    pub revoke: Option<&'static str>,
    /// Extra fixed query parameters for the authorization request.
    pub extra: &'static [(&'static str, &'static str)],
}

/// One API the agent can call with a linked account.
#[derive(Debug, Clone, Copy)]
pub struct Connector {
    /// Stable id, used in `connectors.call`.
    pub id: &'static str,
    /// Name shown to the person.
    pub name: &'static str,
    /// `https://host/prefix`; every call stays below it.
    pub api_base: &'static str,
    /// OAuth scopes the connector needs.
    pub scopes: &'static [&'static str],
}

/// A provider the person can link.
#[derive(Debug, Clone, Copy)]
pub struct Provider {
    /// Stable id.
    pub id: &'static str,
    /// Name shown to the person.
    pub name: &'static str,
    /// Where the person signs in on the web.
    pub sign_in_url: &'static str,
    /// Hosts (exact or any subdomain) that belong to the account.
    pub hosts: &'static [&'static str],
    /// `(url, cookie name)` whose presence means "signed in on the web".
    pub session_cookie: (&'static str, &'static str),
    /// What the agent can do once linked; shown in the consent warning.
    pub control: &'static str,
    /// API sign-in, for providers with connectors.
    pub oauth: Option<OAuth>,
    /// APIs available after an API sign-in.
    pub connectors: &'static [Connector],
}

/// The built-in providers.
pub fn catalog() -> &'static [Provider] {
    const CATALOG: &[Provider] = &[
        Provider {
            id: "google",
            name: "Google",
            sign_in_url: "https://accounts.google.com/",
            hosts: &["google.com", "youtube.com", "gmail.com"],
            session_cookie: ("https://accounts.google.com/", "SID"),
            control: "read, send and delete Gmail; read, change and delete Drive files and Photos; act as you on every Google site",
            oauth: Some(OAuth {
                authorize: "https://accounts.google.com/o/oauth2/v2/auth",
                token: "https://oauth2.googleapis.com/token",
                revoke: Some("https://oauth2.googleapis.com/revoke"),
                extra: &[("access_type", "offline"), ("prompt", "consent")],
            }),
            connectors: &[
                Connector { id: "drive", name: "Google Drive", api_base: "https://www.googleapis.com/drive/v3", scopes: &["https://www.googleapis.com/auth/drive"] },
                Connector { id: "gmail", name: "Gmail", api_base: "https://gmail.googleapis.com/gmail/v1", scopes: &["https://mail.google.com/"] },
                Connector { id: "photos", name: "Google Photos", api_base: "https://photoslibrary.googleapis.com/v1", scopes: &["https://www.googleapis.com/auth/photoslibrary.appendonly", "https://www.googleapis.com/auth/photoslibrary.readonly.appcreateddata"] },
                Connector { id: "calendar", name: "Google Calendar", api_base: "https://www.googleapis.com/calendar/v3", scopes: &["https://www.googleapis.com/auth/calendar"] },
            ],
        },
        Provider {
            id: "github",
            name: "GitHub",
            sign_in_url: "https://github.com/login",
            hosts: &["github.com"],
            session_cookie: ("https://github.com/", "user_session"),
            control: "read and change your repositories, issues and pull requests; act as you on github.com",
            oauth: Some(OAuth {
                authorize: "https://github.com/login/oauth/authorize",
                token: "https://github.com/login/oauth/access_token",
                revoke: None,
                extra: &[],
            }),
            connectors: &[Connector { id: "github", name: "GitHub API", api_base: "https://api.github.com", scopes: &["repo", "read:user"] }],
        },
        Provider {
            id: "yandex",
            name: "Yandex",
            sign_in_url: "https://passport.yandex.com/auth",
            hosts: &["yandex.com", "yandex.ru", "ya.ru"],
            session_cookie: ("https://passport.yandex.com/", "Session_id"),
            control: "read and change your Yandex Disk files; act as you on every Yandex site",
            oauth: Some(OAuth {
                authorize: "https://oauth.yandex.com/authorize",
                token: "https://oauth.yandex.com/token",
                revoke: Some("https://oauth.yandex.com/revoke_token"),
                extra: &[],
            }),
            connectors: &[Connector { id: "disk", name: "Yandex Disk", api_base: "https://cloud-api.yandex.net/v1/disk", scopes: &["cloud_api:disk.read", "cloud_api:disk.write"] }],
        },
        Provider {
            id: "linkedin",
            name: "LinkedIn",
            sign_in_url: "https://www.linkedin.com/login",
            hosts: &["linkedin.com"],
            session_cookie: ("https://www.linkedin.com/", "li_at"),
            control: "apply for jobs, send messages and connection requests, and change your profile as you on linkedin.com",
            oauth: None,
            connectors: &[],
        },
    ];
    CATALOG
}

/// Looks a provider up by id.
pub fn provider(id: &str) -> Result<&'static Provider, Error> {
    catalog()
        .iter()
        .find(|provider| provider.id == id)
        .ok_or(Error::Unknown("provider"))
}

/// The provider a web host belongs to (`mail.google.com` -> Google).
pub fn provider_for_host(host: &str) -> Option<&'static Provider> {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    catalog().iter().find(|provider| {
        provider
            .hosts
            .iter()
            .any(|own| host == *own || host.ends_with(&format!(".{own}")))
    })
}

/// The sentence the person must accept before an account is linked.
pub fn consent_warning(provider: &Provider) -> String {
    format!(
        "If you link {name}, AI agents in this browser get 100% control of this account. \
         They can do anything you can do while signed in: {control}. \
         They act without asking you each time. Link only an account you are willing to hand over, \
         and unlink it here to take the access back.",
        name = provider.name,
        control = provider.control
    )
}

/// A site the person added themselves. It has a web session only: the agent
/// uses it through its pages, as with LinkedIn.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CustomSite {
    /// Stable id, derived from the host.
    pub id: String,
    /// Name shown to the person.
    pub name: String,
    /// Where the person signs in.
    pub sign_in_url: String,
    /// The host the account lives on (with its subdomains).
    pub host: String,
    /// Name of the cookie whose presence means "signed in".
    pub cookie: String,
    /// The site's API, when the person described one.
    #[serde(default)]
    pub api: Option<Api>,
}

impl CustomSite {
    /// Checks what the person typed. The address must be `https` (plain
    /// `http` only for this computer, for development).
    pub fn new(name: &str, sign_in_url: &str, cookie: &str) -> Result<Self, Error> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 40 || name.chars().any(char::is_control) {
            return Err(Error::BadSite("the name must be 1 to 40 characters"));
        }
        if cookie.is_empty()
            || cookie.len() > 64
            || !cookie
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-.".contains(&b))
        {
            return Err(Error::BadSite("the session cookie name is not valid"));
        }
        let (scheme, rest) = sign_in_url.split_once("://").ok_or(Error::BadSite(
            "the sign-in address must start with https://",
        ))?;
        let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
        let (host, port) = authority.split_once(':').unwrap_or((authority, ""));
        let host = host.to_ascii_lowercase();
        let local = matches!(host.as_str(), "127.0.0.1" | "localhost");
        if !(scheme == "https" || scheme == "http" && local) {
            return Err(Error::BadSite(
                "the sign-in address must start with https://",
            ));
        }
        if host.is_empty()
            || sign_in_url.len() > 512
            || !host
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
            || !port.bytes().all(|b| b.is_ascii_digit())
            || !(local || host.contains('.'))
        {
            return Err(Error::BadSite("the sign-in address has no valid host"));
        }
        if provider_for_host(&host).is_some() {
            return Err(Error::BadSite("this site is already in the list"));
        }
        Ok(Self {
            id: authority.to_ascii_lowercase().replace(['.', ':'], "-"),
            name: name.to_string(),
            sign_in_url: sign_in_url.to_string(),
            host,
            cookie: cookie.to_string(),
            api: None,
        })
    }

    /// Whether a web host belongs to this site.
    pub fn owns(&self, host: &str) -> bool {
        let host = host.trim_end_matches('.').to_ascii_lowercase();
        host == self.host || host.ends_with(&format!(".{}", self.host))
    }
}

/// The consent sentence for a site the person added.
pub fn custom_consent_warning(site: &CustomSite) -> String {
    format!(
        "If you link {name}, AI agents in this browser get 100% control of this account.          They can do anything you can do while signed in on {host}, and they act without asking you each time.          Link only an account you are willing to hand over, and unlink it here to take the access back.",
        name = site.name,
        host = site.host
    )
}

fn base64url(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let block = chunk
            .iter()
            .fold(0u32, |acc, byte| acc << 8 | u32::from(*byte))
            << (8 * (3 - chunk.len()));
        for index in 0..=chunk.len() {
            out.push(ALPHABET[(block >> (18 - 6 * index) & 63) as usize] as char);
        }
    }
    out
}

/// 32 random bytes from the operating system, as 43 URL-safe characters.
pub fn random_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("the operating system random source is unavailable");
    base64url(&bytes)
}

fn percent(text: &str) -> String {
    text.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (byte as char).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect()
}

fn unpercent(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' => {
                let hex = std::str::from_utf8(bytes.get(index + 1..index + 3)?).ok()?;
                out.push(u8::from_str_radix(hex, 16).ok()?);
                index += 3;
            }
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            other => {
                out.push(other);
                index += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

fn same(left: &str, right: &str) -> bool {
    // Compare every byte so the time taken does not reveal a matching prefix.
    left.len() == right.len()
        && left
            .bytes()
            .zip(right.bytes())
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0
}

/// The API side of an account: where to sign in and what can be called.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Api {
    /// Authorization endpoint the person is sent to.
    pub authorize: String,
    /// Token endpoint (code exchange and refresh).
    pub token: String,
    /// Revocation endpoint, when there is one.
    #[serde(default)]
    pub revoke: Option<String>,
    /// Extra fixed query parameters for the authorization request.
    #[serde(default)]
    pub extra: Vec<(String, String)>,
    /// APIs available after sign-in.
    pub connectors: Vec<ApiConnector>,
}

/// One callable API of an [`Api`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ApiConnector {
    /// Stable id, used in `connectors.call`.
    pub id: String,
    /// Name shown to the person.
    pub name: String,
    /// `https://host/prefix`; every call stays below it.
    pub api_base: String,
    /// OAuth scopes the connector needs.
    pub scopes: Vec<String>,
}

fn web_address(url: &str) -> Result<(), Error> {
    let (scheme, rest) = url
        .split_once("://")
        .ok_or(Error::BadSite("addresses must start with https://"))?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority.split(':').next().unwrap_or("");
    let local = matches!(host, "127.0.0.1" | "localhost");
    if !(scheme == "https" || scheme == "http" && local) {
        return Err(Error::BadSite("addresses must start with https://"));
    }
    if host.is_empty()
        || url.len() > 512
        || url
            .bytes()
            .any(|b| b <= b' ' || b >= 0x7F || b == b'\\' || b == b'@')
    {
        return Err(Error::BadSite("an address is not valid"));
    }
    Ok(())
}

impl Api {
    /// An API the person describes themselves (their own OAuth application
    /// on any service). One connector, called `api`.
    pub fn custom(
        authorize: &str,
        token: &str,
        api_base: &str,
        scopes: &str,
    ) -> Result<Self, Error> {
        for url in [authorize, token, api_base] {
            web_address(url)?;
        }
        if authorize.contains(['?', '#']) || api_base.contains(['?', '#']) {
            return Err(Error::BadSite(
                "the authorization and API addresses may not contain ? or #",
            ));
        }
        let scopes: Vec<String> = scopes.split_whitespace().map(str::to_string).collect();
        if scopes.len() > 30 || scopes.iter().any(|scope| scope.len() > 200) {
            return Err(Error::BadSite("too many or too long scopes"));
        }
        Ok(Self {
            authorize: authorize.to_string(),
            token: token.to_string(),
            revoke: None,
            extra: Vec::new(),
            connectors: vec![ApiConnector {
                id: "api".into(),
                name: "API".into(),
                api_base: api_base.trim_end_matches('/').to_string(),
                scopes,
            }],
        })
    }
}

impl Provider {
    /// The API side of a built-in provider, if it has one.
    pub fn api(&self) -> Option<Api> {
        let oauth = self.oauth?;
        Some(Api {
            authorize: oauth.authorize.into(),
            token: oauth.token.into(),
            revoke: oauth.revoke.map(str::to_string),
            extra: oauth
                .extra
                .iter()
                .map(|(key, value)| (key.to_string(), value.to_string()))
                .collect(),
            connectors: self
                .connectors
                .iter()
                .map(|connector| ApiConnector {
                    id: connector.id.into(),
                    name: connector.name.into(),
                    api_base: connector.api_base.into(),
                    scopes: connector
                        .scopes
                        .iter()
                        .map(|scope| scope.to_string())
                        .collect(),
                })
                .collect(),
        })
    }
}

/// One sign-in attempt. Created when the person presses "Connect", consumed
/// by [`Attempt::finish`], which takes `self` so an answer can be used once.
pub struct Attempt {
    api: Api,
    client_id: String,
    port: u16,
    state: String,
    verifier: String,
    scopes: Vec<String>,
}

impl std::fmt::Debug for Attempt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Attempt(port {})", self.port)
    }
}

impl Attempt {
    /// Starts an attempt for a built-in provider, answered on `127.0.0.1:port`.
    pub fn new(provider_id: &str, client_id: &str, port: u16) -> Result<Self, Error> {
        let api = provider(provider_id)?
            .api()
            .ok_or(Error::Unknown("API sign-in for this provider"))?;
        Self::with_api(api, client_id, port)
    }

    /// Starts an attempt for any [`Api`].
    pub fn with_api(api: Api, client_id: &str, port: u16) -> Result<Self, Error> {
        if client_id.is_empty() || client_id.len() > 512 || client_id.chars().any(char::is_control)
        {
            return Err(Error::BadSite("a client id is required"));
        }
        let mut scopes: Vec<String> = Vec::new();
        for scope in api
            .connectors
            .iter()
            .flat_map(|connector| connector.scopes.iter())
        {
            if !scopes.contains(scope) {
                scopes.push(scope.clone());
            }
        }
        Ok(Self {
            api,
            client_id: client_id.to_string(),
            port,
            state: random_token(),
            verifier: random_token(),
            scopes,
        })
    }

    /// The exact redirect address registered for this attempt.
    pub fn redirect_uri(&self) -> String {
        format!("http://127.0.0.1:{}/callback", self.port)
    }

    /// Where to send the person.
    pub fn authorize_url(&self) -> String {
        let challenge = base64url(&Sha256::digest(self.verifier.as_bytes()));
        let mut url = format!(
            "{}?response_type=code&client_id={}&redirect_uri={}&scope={}&state={}&code_challenge={}&code_challenge_method=S256",
            self.api.authorize,
            percent(&self.client_id),
            percent(&self.redirect_uri()),
            percent(&self.scopes.join(" ")),
            self.state,
            challenge,
        );
        for (key, value) in &self.api.extra {
            url.push_str(&format!("&{}={}", percent(key), percent(value)));
        }
        url
    }

    /// Checks the request the provider's redirect made to our loopback
    /// listener and returns the body for the token endpoint.
    ///
    /// `host` is the request's `Host` header and `target` its request target
    /// (`/callback?code=...&state=...`).
    pub fn finish(
        self,
        host: &str,
        target: &str,
        client_secret: Option<&str>,
    ) -> Result<TokenRequest, Error> {
        if host != format!("127.0.0.1:{}", self.port) {
            return Err(Error::WrongRedirect);
        }
        let (path, query) = target.split_once('?').unwrap_or((target, ""));
        if path != "/callback" {
            return Err(Error::WrongRedirect);
        }
        let mut code = Vec::new();
        let mut state = Vec::new();
        let mut error = None;
        for pair in query.split('&').filter(|pair| !pair.is_empty()) {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            let value = unpercent(value).ok_or(Error::BadCode)?;
            match key {
                "code" => code.push(value),
                "state" => state.push(value),
                "error" => error = Some(value),
                _ => {}
            }
        }
        // `state` first: an answer that is not ours tells us nothing we trust.
        if state.len() != 1 || !same(&state[0], &self.state) {
            return Err(Error::StateMismatch);
        }
        if let Some(error) = error {
            return Err(Error::Denied(plain_word(&error)));
        }
        if code.len() != 1 || code[0].is_empty() || code[0].len() > 2048 {
            return Err(Error::BadCode);
        }
        let mut body = format!(
            "grant_type=authorization_code&code={}&redirect_uri={}&client_id={}&code_verifier={}",
            percent(&code[0]),
            percent(&self.redirect_uri()),
            percent(&self.client_id),
            self.verifier,
        );
        if let Some(secret) = client_secret.filter(|secret| !secret.is_empty()) {
            body.push_str(&format!("&client_secret={}", percent(secret)));
        }
        Ok(TokenRequest {
            url: self.api.token.clone(),
            body,
            scopes: self.scopes,
        })
    }
}

fn plain_word(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .take(60)
        .collect()
}

/// A `POST` (form-encoded, `Accept: application/json`) for the token endpoint.
pub struct TokenRequest {
    /// Token endpoint.
    pub url: String,
    /// Form body; contains the code and the PKCE verifier.
    pub body: String,
    /// Scopes that were asked for.
    pub scopes: Vec<String>,
}

/// Body of a refresh request.
pub fn refresh_request(api: &Api, tokens: &Tokens) -> Result<TokenRequest, Error> {
    let refresh = tokens
        .refresh_token
        .as_deref()
        .ok_or(Error::BadTokenResponse)?;
    let mut body = format!(
        "grant_type=refresh_token&refresh_token={}&client_id={}",
        percent(refresh),
        percent(&tokens.client_id)
    );
    if let Some(secret) = tokens
        .client_secret
        .as_deref()
        .filter(|secret| !secret.is_empty())
    {
        body.push_str(&format!("&client_secret={}", percent(secret)));
    }
    Ok(TokenRequest {
        url: api.token.clone(),
        body,
        scopes: tokens.scopes.clone(),
    })
}

/// Reads a token endpoint answer. `now` is seconds since the Unix epoch.
pub fn parse_token_response(
    body: &str,
    scopes: Vec<String>,
    now: u64,
    previous: Option<&Tokens>,
) -> Result<Tokens, Error> {
    // GitHub answers form-encoded unless asked for JSON; accept both.
    let value: serde_json::Value = serde_json::from_str(body).or_else(|_| {
        let pairs: serde_json::Map<String, serde_json::Value> = body
            .split('&')
            .filter_map(|pair| pair.split_once('='))
            .filter_map(|(key, value)| {
                Some((
                    key.to_string(),
                    serde_json::Value::String(unpercent(value)?),
                ))
            })
            .collect();
        if pairs.contains_key("access_token") || pairs.contains_key("error") {
            Ok(serde_json::Value::Object(pairs))
        } else {
            Err(Error::BadTokenResponse)
        }
    })?;
    if let Some(error) = value.get("error").and_then(|error| error.as_str()) {
        return Err(Error::Denied(plain_word(error)));
    }
    let access = value
        .get("access_token")
        .and_then(|token| token.as_str())
        .filter(|token| !token.is_empty())
        .ok_or(Error::BadTokenResponse)?;
    let kind = value
        .get("token_type")
        .and_then(|kind| kind.as_str())
        .unwrap_or("bearer");
    if !kind.eq_ignore_ascii_case("bearer") {
        return Err(Error::BadTokenResponse);
    }
    let seconds = value
        .get("expires_in")
        .and_then(|seconds| seconds.as_u64().or_else(|| seconds.as_str()?.parse().ok()));
    Ok(Tokens {
        access_token: access.to_string(),
        // A refresh answer may omit the refresh token: keep the one we have.
        refresh_token: value
            .get("refresh_token")
            .and_then(|token| token.as_str())
            .map(str::to_string)
            .or_else(|| previous.and_then(|tokens| tokens.refresh_token.clone())),
        expires_at: seconds.map(|seconds| now + seconds),
        scopes,
        linked_at: previous.map_or(now, |tokens| tokens.linked_at),
        client_id: previous
            .map(|tokens| tokens.client_id.clone())
            .unwrap_or_default(),
        client_secret: previous.and_then(|tokens| tokens.client_secret.clone()),
    })
}

/// Methods a connector call may use.
pub const CONNECTOR_METHODS: [&str; 5] = ["GET", "POST", "PUT", "PATCH", "DELETE"];

/// Builds the address of a call to a built-in provider's connector.
pub fn connector_url(
    provider_id: &str,
    connector_id: &str,
    method: &str,
    path: &str,
) -> Result<String, Error> {
    let api = provider(provider_id)?
        .api()
        .ok_or(Error::Unknown("connector"))?;
    api_url(&api, connector_id, method, path)
}

/// Builds the address of a connector call. `path` is relative to the
/// connector's API (`/files?pageSize=5`) and cannot leave it.
pub fn api_url(api: &Api, connector_id: &str, method: &str, path: &str) -> Result<String, Error> {
    let connector = api
        .connectors
        .iter()
        .find(|connector| connector.id == connector_id)
        .ok_or(Error::Unknown("connector"))?;
    if !CONNECTOR_METHODS.contains(&method) {
        return Err(Error::BadConnectorCall(
            "method must be GET, POST, PUT, PATCH or DELETE",
        ));
    }
    let (route, _query) = path.split_once('?').unwrap_or((path, ""));
    if !route.starts_with('/') || route.starts_with("//") {
        return Err(Error::BadConnectorCall("path must start with one /"));
    }
    if path.len() > 4096
        || path
            .bytes()
            .any(|byte| byte <= b' ' || byte == b'\\' || byte == b'#' || byte >= 0x7F)
    {
        return Err(Error::BadConnectorCall(
            "path has characters that are not allowed",
        ));
    }
    // Also catches the encoded forms a server might decode before routing.
    let lowered = route.to_ascii_lowercase();
    if lowered
        .split('/')
        .any(|segment| matches!(segment, ".." | "%2e%2e" | ".%2e" | "%2e."))
        || route.contains('@')
    {
        return Err(Error::BadConnectorCall("path may not climb out of the API"));
    }
    Ok(format!("{}{}", connector.api_base, path))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attempt() -> Attempt {
        Attempt::new("google", "client-1", 49152).unwrap()
    }

    fn answer(attempt: &Attempt, code: &str) -> String {
        format!("/callback?code={code}&state={}", attempt.state)
    }

    #[test]
    fn base64url_matches_the_rfc_vectors() {
        assert_eq!(base64url(b""), "");
        assert_eq!(base64url(b"f"), "Zg");
        assert_eq!(base64url(b"fo"), "Zm8");
        assert_eq!(base64url(b"foo"), "Zm9v");
        assert_eq!(base64url(&[0xfb, 0xff]), "-_8");
        // RFC 7636 appendix B.
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(
            base64url(&Sha256::digest(verifier.as_bytes())),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn catalog_is_https_only_and_connectors_stay_on_their_host() {
        for provider in catalog() {
            assert!(provider.sign_in_url.starts_with("https://"));
            assert!(provider.session_cookie.0.starts_with("https://"));
            if let Some(oauth) = provider.oauth {
                for url in [Some(oauth.authorize), Some(oauth.token), oauth.revoke]
                    .into_iter()
                    .flatten()
                {
                    assert!(url.starts_with("https://"), "{url}");
                }
            }
            assert_eq!(provider.connectors.is_empty(), provider.oauth.is_none());
            for connector in provider.connectors {
                assert!(
                    connector.api_base.starts_with("https://")
                        && !connector.api_base.ends_with('/')
                );
            }
            assert!(consent_warning(provider).contains("100% control"));
        }
        assert_eq!(provider_for_host("mail.google.com").unwrap().id, "google");
        assert_eq!(provider_for_host("GitHub.com.").unwrap().id, "github");
        assert!(provider_for_host("notgoogle.com").is_none());
        assert!(provider_for_host("google.com.evil.example").is_none());
    }

    #[test]
    fn the_authorize_url_always_uses_s256_and_a_loopback_redirect() {
        let attempt = attempt();
        let url = attempt.authorize_url();
        assert!(url.starts_with("https://accounts.google.com/o/oauth2/v2/auth?response_type=code&"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A49152%2Fcallback"));
        assert!(url.contains(&format!("state={}", attempt.state)));
        // The verifier itself never travels in the address.
        assert!(!url.contains(&attempt.verifier));
        // Each attempt has its own secrets.
        let other = super::Attempt::new("google", "client-1", 49152).unwrap();
        assert_ne!(other.state, attempt.state);
        assert_ne!(other.verifier, attempt.verifier);
        assert_eq!(attempt.state.len(), 43);
    }

    #[test]
    fn a_good_answer_yields_a_token_request_with_the_verifier() {
        let attempt = attempt();
        let verifier = attempt.verifier.clone();
        let target = answer(&attempt, "4%2Fabc");
        let request = attempt.finish("127.0.0.1:49152", &target, None).unwrap();
        assert_eq!(request.url, "https://oauth2.googleapis.com/token");
        assert!(request.body.contains("code=4%2Fabc"));
        assert!(request.body.contains(&format!("code_verifier={verifier}")));
        assert!(!request.body.contains("client_secret"));
    }

    #[test]
    fn forged_or_tampered_answers_are_refused() {
        // Someone else's attempt (CSRF): wrong, missing or doubled state.
        assert_eq!(
            attempt()
                .finish("127.0.0.1:49152", "/callback?code=x&state=guess", None)
                .err(),
            Some(Error::StateMismatch)
        );
        assert_eq!(
            attempt()
                .finish("127.0.0.1:49152", "/callback?code=x", None)
                .err(),
            Some(Error::StateMismatch)
        );
        let a = attempt();
        let doubled = format!("/callback?code=x&state={0}&state={0}", a.state);
        assert_eq!(
            a.finish("127.0.0.1:49152", &doubled, None).err(),
            Some(Error::StateMismatch)
        );
        // Redirect tampering: other host name, other port, other path.
        for (host, path) in [
            ("localhost:49152", "/callback"),
            ("127.0.0.1:49153", "/callback"),
            ("evil.example", "/callback"),
            ("127.0.0.1:49152", "/callback/"),
            ("127.0.0.1:49152", "/other"),
        ] {
            let a = attempt();
            let target = format!("{path}?code=x&state={}", a.state);
            assert_eq!(
                a.finish(host, &target, None).err(),
                Some(Error::WrongRedirect),
                "{host}{path}"
            );
        }
        // Code injection: two codes, no code.
        let a = attempt();
        let two = format!("/callback?code=x&code=y&state={}", a.state);
        assert_eq!(
            a.finish("127.0.0.1:49152", &two, None).err(),
            Some(Error::BadCode)
        );
        let a = attempt();
        let none = format!("/callback?state={}", a.state);
        assert_eq!(
            a.finish("127.0.0.1:49152", &none, None).err(),
            Some(Error::BadCode)
        );
        // The person said no; the provider's text is reduced to a plain word.
        let a = attempt();
        let denied = format!(
            "/callback?error=access_denied%3Cscript%3E&state={}",
            a.state
        );
        assert_eq!(
            a.finish("127.0.0.1:49152", &denied, None).err(),
            Some(Error::Denied("access_deniedscript".into()))
        );
    }

    #[test]
    fn token_answers_must_be_bearer_tokens() {
        let scopes = vec!["s".to_string()];
        let tokens = parse_token_response(
            r#"{"access_token":"at","refresh_token":"rt","expires_in":3600,"token_type":"Bearer"}"#,
            scopes.clone(),
            1000,
            None,
        )
        .unwrap();
        assert_eq!((tokens.expires_at, tokens.linked_at), (Some(4600), 1000));
        // A refresh keeps the old refresh token and the original link time.
        let renewed = parse_token_response(
            r#"{"access_token":"at2","expires_in":10}"#,
            scopes.clone(),
            5000,
            Some(&tokens),
        )
        .unwrap();
        assert_eq!(renewed.refresh_token.as_deref(), Some("rt"));
        assert_eq!(renewed.linked_at, 1000);
        assert_eq!(
            parse_token_response(
                r#"{"access_token":"at","token_type":"mac"}"#,
                scopes.clone(),
                0,
                None
            )
            .err(),
            Some(Error::BadTokenResponse)
        );
        assert_eq!(
            parse_token_response(r#"{"error":"invalid_grant"}"#, scopes.clone(), 0, None).err(),
            Some(Error::Denied("invalid_grant".into()))
        );
        assert_eq!(
            parse_token_response("<html>", scopes, 0, None).err(),
            Some(Error::BadTokenResponse)
        );
        // Tokens never show up in debug output (logs, panics, error text).
        let shown = format!("{tokens:?}");
        assert!(!shown.contains("rt"), "{shown}");
        assert!(shown.contains("redacted"));
    }

    #[test]
    fn custom_sites_are_validated() {
        let site = CustomSite::new(" Jobs ", "https://Jobs.Example.com/login?x=1", "sid").unwrap();
        assert_eq!(
            (site.id.as_str(), site.host.as_str(), site.name.as_str()),
            ("jobs-example-com", "jobs.example.com", "Jobs")
        );
        assert!(
            site.owns("app.jobs.example.com")
                && !site.owns("example.com")
                && !site.owns("jobs.example.com.evil.io")
        );
        assert!(custom_consent_warning(&site).contains("100% control"));
        assert!(CustomSite::new("Dev", "http://127.0.0.1:8080/login", "sid").is_ok());
        for (name, url, cookie) in [
            ("", "https://a.example/", "sid"),
            ("A", "http://a.example/", "sid"),
            ("A", "javascript:alert(1)", "sid"),
            ("A", "https://user@a.example/", "sid"),
            ("A", "https://a.example:x/", "sid"),
            ("A", "https://intranet/", "sid"),
            ("A", "https://a.example/", "s id"),
            ("A", "https://a.example/", ""),
            ("A", "https://mail.google.com/", "SID"),
        ] {
            assert!(
                matches!(CustomSite::new(name, url, cookie), Err(Error::BadSite(_))),
                "{name} {url} {cookie}"
            );
        }
    }

    #[test]
    fn custom_apis_are_validated_and_work_like_built_in_ones() {
        let api = Api::custom(
            "http://127.0.0.1:7000/authorize",
            "http://127.0.0.1:7000/token",
            "http://127.0.0.1:7000/api/",
            "read write read",
        )
        .unwrap();
        assert_eq!(api.connectors[0].api_base, "http://127.0.0.1:7000/api");
        assert_eq!(
            api_url(&api, "api", "GET", "/items?x=1").unwrap(),
            "http://127.0.0.1:7000/api/items?x=1"
        );
        let attempt = Attempt::with_api(api.clone(), "client", 50000).unwrap();
        assert!(
            attempt.authorize_url().contains("scope=read%20write&"),
            "{}",
            attempt.authorize_url()
        );
        assert!(Attempt::with_api(api, "", 1).is_err());
        for (authorize, token, base) in [
            (
                "http://idp.example/authorize",
                "https://idp.example/token",
                "https://api.example",
            ),
            (
                "https://idp.example/authorize",
                "ftp://idp.example/token",
                "https://api.example",
            ),
            (
                "https://idp.example/authorize?x=1",
                "https://idp.example/token",
                "https://api.example",
            ),
            (
                "https://idp.example/authorize",
                "https://idp.example/token",
                "https://user@api.example",
            ),
        ] {
            assert!(
                Api::custom(authorize, token, base, "").is_err(),
                "{authorize} {token} {base}"
            );
        }
        // GitHub's form-encoded token answer.
        let tokens = parse_token_response(
            "access_token=gho_x&scope=repo&token_type=bearer",
            vec![],
            5,
            None,
        )
        .unwrap();
        assert_eq!(tokens.expires_at, None);
    }

    #[test]
    fn connector_calls_cannot_leave_their_api() {
        assert_eq!(
            connector_url(
                "google",
                "drive",
                "GET",
                "/files?pageSize=5&q=name%20contains%20'a'"
            )
            .unwrap(),
            "https://www.googleapis.com/drive/v3/files?pageSize=5&q=name%20contains%20'a'"
        );
        assert_eq!(
            connector_url("github", "github", "POST", "/repos/o/r/issues").unwrap(),
            "https://api.github.com/repos/o/r/issues"
        );
        for bad in [
            "files",
            "//evil.example/x",
            "/../../oauth2/v1/tokeninfo",
            "/a/%2E%2e/b",
            "/x\\y",
            "/x y",
            "/a\r\nHost: evil",
            "/@evil.example",
            "/a#b",
            "https://evil.example/",
        ] {
            assert!(
                matches!(
                    connector_url("google", "drive", "GET", bad),
                    Err(Error::BadConnectorCall(_))
                ),
                "{bad}"
            );
        }
        assert!(matches!(
            connector_url("google", "drive", "TRACE", "/files"),
            Err(Error::BadConnectorCall(_))
        ));
        assert_eq!(
            connector_url("google", "nope", "GET", "/x").err(),
            Some(Error::Unknown("connector"))
        );
        assert_eq!(
            connector_url("linkedin", "x", "GET", "/x").err(),
            Some(Error::Unknown("connector"))
        );
        assert_eq!(
            Attempt::new("linkedin", "c", 1)
                .err()
                .map(|e| e.to_string()),
            Some("unknown API sign-in for this provider".into())
        );
    }
}
