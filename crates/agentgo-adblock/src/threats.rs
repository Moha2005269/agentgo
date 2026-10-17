//! Known malware and phishing sites, checked before every page opens.
//!
//! Filter lists cancel a dangerous site's subresources but never the page
//! itself, so a link straight to a phishing page would still load. This table
//! (`data/threats.txt`, built by `scripts/dev/build-threats.js` from URLhaus,
//! uBlock's Badware list and the Phishing URL Blocklist) answers the question
//! for top-level pages, offline: no address ever leaves the computer.

use std::collections::HashMap;
use std::sync::OnceLock;

const THREATS: &str = include_str!("../data/threats.txt");

/// Why a site is dangerous.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Threat {
    /// Spreads malware, fake updates or scams.
    Malware,
    /// Imitates another site to steal sign-ins or payment details.
    Phishing,
}

impl Threat {
    /// Stable lowercase name (`malware`, `phishing`).
    pub fn code(self) -> &'static str {
        match self {
            Threat::Malware => "malware",
            Threat::Phishing => "phishing",
        }
    }
}

fn table() -> &'static HashMap<&'static str, Threat> {
    static TABLE: OnceLock<HashMap<&'static str, Threat>> = OnceLock::new();
    TABLE.get_or_init(|| {
        THREATS
            .lines()
            .filter_map(|line| {
                let (kind, host) = line.split_once(' ')?;
                let threat = match kind {
                    "m" => Threat::Malware,
                    "p" => Threat::Phishing,
                    _ => return None,
                };
                Some((host.trim(), threat))
            })
            .collect()
    })
}

/// The lowercase host of an `http(s)` address, without user info or port.
fn host_of(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let authority = authority.rsplit('@').next()?;
    let host = if authority.starts_with('[') {
        authority.split_inclusive(']').next()?
    } else {
        authority.split(':').next()?
    };
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    (!host.is_empty()).then_some(host)
}

/// The threat a page address is known for, matching its host and each parent
/// domain (a listed domain covers its subdomains, never the other way round).
pub fn threat_for_url(url: &str) -> Option<Threat> {
    let host = host_of(url)?;
    let table = table();
    let mut name = host.as_str();
    loop {
        if let Some(threat) = table.get(name) {
            return Some(*threat);
        }
        match name.split_once('.') {
            Some((_, parent)) if parent.contains('.') => name = parent,
            _ => return None,
        }
    }
}

/// How many hosts the table holds.
pub fn threat_count() -> usize {
    table().len()
}

/// A site that imitates a well-known one (found by its address alone, so it
/// also catches phishing pages too new for any list).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lookalike {
    /// The brand it imitates, as people know it ("PayPal").
    pub brand: &'static str,
    /// What gave it away: `homograph` (look-alike letters from another
    /// alphabet), `mixed-script` (letters from several alphabets in one
    /// name), `brand-subdomain` (`paypal.com.example.top`) or `brand-lure`
    /// (`paypal-login-verify.com`).
    pub reason: &'static str,
}

/// Brands phishing imitates most, with every domain that really is theirs.
/// A brand's own domains (and their subdomains) are never flagged.
const BRANDS: &[(&str, &str, &[&str])] = &[
    (
        "paypal",
        "PayPal",
        &[
            "paypal.com",
            "paypal.me",
            "paypalobjects.com",
            "paypal-community.com",
        ],
    ),
    (
        "apple",
        "Apple",
        &[
            "apple.com",
            "icloud.com",
            "apple.news",
            "me.com",
            "mzstatic.com",
            "applecard.apple",
        ],
    ),
    ("icloud", "iCloud", &["icloud.com", "apple.com"]),
    (
        "microsoft",
        "Microsoft",
        &[
            "microsoft.com",
            "live.com",
            "office.com",
            "microsoftonline.com",
            "outlook.com",
            "sharepoint.com",
            "msn.com",
            "azure.com",
            "windows.com",
            "xbox.com",
            "bing.com",
            "skype.com",
            "microsoft365.com",
            "office365.com",
        ],
    ),
    (
        "office365",
        "Microsoft 365",
        &[
            "office.com",
            "microsoft.com",
            "office365.com",
            "microsoftonline.com",
        ],
    ),
    (
        "outlook",
        "Outlook",
        &["outlook.com", "live.com", "office.com", "microsoft.com"],
    ),
    (
        "google",
        "Google",
        &[
            "google.com",
            "gmail.com",
            "youtube.com",
            "googleusercontent.com",
            "gstatic.com",
            "googleapis.com",
            "withgoogle.com",
            "google.co.uk",
            "google.de",
            "google.fr",
            "google.co.jp",
            "google.com.au",
            "google.ca",
            "google.co.in",
            "google.com.br",
            "google.es",
            "google.it",
            "google.nl",
            "google.ru",
            "google.com.my",
            "google.com.sa",
            "google.ae",
            "blog.google",
            "g.co",
            "goo.gl",
        ],
    ),
    ("gmail", "Gmail", &["gmail.com", "google.com"]),
    (
        "amazon",
        "Amazon",
        &[
            "amazon.com",
            "amazon.co.uk",
            "amazon.de",
            "amazon.fr",
            "amazon.it",
            "amazon.es",
            "amazon.ca",
            "amazon.co.jp",
            "amazon.in",
            "amazon.com.au",
            "amazon.com.br",
            "amazon.nl",
            "amazon.sa",
            "amazon.ae",
            "amazon.sg",
            "amazonaws.com",
            "aws.amazon.com",
            "amazon.dev",
            "a2z.com",
            "media-amazon.com",
        ],
    ),
    (
        "facebook",
        "Facebook",
        &[
            "facebook.com",
            "fb.com",
            "fb.me",
            "meta.com",
            "facebook.net",
            "fbcdn.net",
            "messenger.com",
        ],
    ),
    (
        "instagram",
        "Instagram",
        &[
            "instagram.com",
            "cdninstagram.com",
            "meta.com",
            "facebook.com",
        ],
    ),
    (
        "whatsapp",
        "WhatsApp",
        &["whatsapp.com", "whatsapp.net", "wa.me", "meta.com"],
    ),
    (
        "netflix",
        "Netflix",
        &["netflix.com", "nflxext.com", "nflximg.net", "netflix.net"],
    ),
    (
        "chase",
        "Chase",
        &["chase.com", "jpmorganchase.com", "jpmorgan.com"],
    ),
    ("wellsfargo", "Wells Fargo", &["wellsfargo.com", "wf.com"]),
    (
        "bankofamerica",
        "Bank of America",
        &["bankofamerica.com", "bofa.com", "ml.com"],
    ),
    ("coinbase", "Coinbase", &["coinbase.com", "cb.com"]),
    (
        "binance",
        "Binance",
        &["binance.com", "binance.us", "bnbchain.org"],
    ),
    ("metamask", "MetaMask", &["metamask.io", "consensys.io"]),
    (
        "steam",
        "Steam",
        &[
            "steampowered.com",
            "steamcommunity.com",
            "steamstatic.com",
            "steamgames.com",
        ],
    ),
    (
        "steamcommunity",
        "Steam",
        &["steamcommunity.com", "steampowered.com"],
    ),
    (
        "dhl",
        "DHL",
        &[
            "dhl.com",
            "dhl.de",
            "dhl.co.uk",
            "dhlparcel.nl",
            "dpdhl.com",
        ],
    ),
    ("fedex", "FedEx", &["fedex.com"]),
    ("usps", "USPS", &["usps.com", "usps.gov"]),
    (
        "adobe",
        "Adobe",
        &["adobe.com", "adobe.io", "adobelogin.com", "typekit.net"],
    ),
    ("docusign", "DocuSign", &["docusign.com", "docusign.net"]),
    (
        "dropbox",
        "Dropbox",
        &["dropbox.com", "dropboxusercontent.com", "db.tt"],
    ),
    (
        "linkedin",
        "LinkedIn",
        &["linkedin.com", "licdn.com", "lnkd.in"],
    ),
    ("roblox", "Roblox", &["roblox.com", "rbxcdn.com"]),
    (
        "discord",
        "Discord",
        &[
            "discord.com",
            "discord.gg",
            "discordapp.com",
            "discord.media",
            "discordapp.net",
        ],
    ),
    (
        "telegram",
        "Telegram",
        &["telegram.org", "t.me", "telegram.me"],
    ),
    (
        "github",
        "GitHub",
        &[
            "github.com",
            "github.io",
            "githubusercontent.com",
            "githubassets.com",
            "github.dev",
        ],
    ),
    (
        "yahoo",
        "Yahoo",
        &["yahoo.com", "yahoo.co.jp", "yimg.com", "aol.com"],
    ),
    (
        "ebay",
        "eBay",
        &[
            "ebay.com",
            "ebay.co.uk",
            "ebay.de",
            "ebay.fr",
            "ebay.it",
            "ebay.es",
            "ebay.com.au",
            "ebay.ca",
            "ebayimg.com",
        ],
    ),
    (
        "hsbc",
        "HSBC",
        &[
            "hsbc.com",
            "hsbc.co.uk",
            "hsbc.com.my",
            "hsbc.com.hk",
            "us.hsbc.com",
        ],
    ),
    (
        "americanexpress",
        "American Express",
        &["americanexpress.com", "aexp.com", "amex.com"],
    ),
    (
        "spotify",
        "Spotify",
        &["spotify.com", "scdn.co", "spotify.link"],
    ),
    (
        "tiktok",
        "TikTok",
        &["tiktok.com", "tiktokv.com", "tiktokcdn.com"],
    ),
    (
        "twitter",
        "X (Twitter)",
        &["twitter.com", "x.com", "t.co", "twimg.com"],
    ),
    ("walmart", "Walmart", &["walmart.com", "walmartimages.com"]),
];

/// Words phishing pages add to a brand name to look official.
const LURES: &[&str] = &[
    "login",
    "log-in",
    "signin",
    "sign-in",
    "logon",
    "secure",
    "security",
    "verify",
    "verification",
    "account",
    "accounts",
    "update",
    "support",
    "help",
    "wallet",
    "recovery",
    "recover",
    "unlock",
    "locked",
    "billing",
    "confirm",
    "auth",
    "authentication",
    "2fa",
    "password",
    "reset",
    "refund",
    "claim",
    "reward",
    "gift",
    "suspended",
    "alert",
    "signon",
    "sso",
    "payment",
    "invoice",
];

/// Letters from other alphabets that look like Latin ones, and the Latin
/// letter they imitate.
fn latin_twin(c: char) -> Option<char> {
    Some(match c {
        // Cyrillic
        'а' => 'a',
        'в' => 'b',
        'е' => 'e',
        'ё' => 'e',
        'к' => 'k',
        'м' => 'm',
        'н' => 'h',
        'о' => 'o',
        'р' => 'p',
        'с' => 'c',
        'т' => 't',
        'у' => 'y',
        'х' => 'x',
        'і' => 'i',
        'ї' => 'i',
        'ј' => 'j',
        'ѕ' => 's',
        'ԁ' => 'd',
        'ӏ' => 'l',
        'һ' => 'h',
        'ԛ' => 'q',
        'ԝ' => 'w',
        'ɡ' => 'g',
        'ս' => 'u',
        // Greek
        'α' => 'a',
        'ο' => 'o',
        'ρ' => 'p',
        'ν' => 'v',
        'τ' => 't',
        'ι' => 'i',
        'κ' => 'k',
        'υ' => 'u',
        'ε' => 'e',
        'χ' => 'x',
        'ω' => 'w',
        'β' => 'b',
        'η' => 'n',
        // Latin look-alikes outside ASCII
        'ı' => 'i',
        'ł' => 'l',
        'ɩ' => 'i',
        'ɑ' => 'a',
        'ȷ' => 'j',
        'ḿ' => 'm',
        'ṁ' => 'm',
        'ọ' => 'o',
        'ö' => 'o',
        'ó' => 'o',
        'ò' => 'o',
        'ô' => 'o',
        'õ' => 'o',
        'á' => 'a',
        'à' => 'a',
        'â' => 'a',
        'ä' => 'a',
        'ã' => 'a',
        'é' => 'e',
        'è' => 'e',
        'ê' => 'e',
        'ë' => 'e',
        'í' => 'i',
        'ì' => 'i',
        'î' => 'i',
        'ï' => 'i',
        'ú' => 'u',
        'ù' => 'u',
        'û' => 'u',
        'ü' => 'u',
        'ç' => 'c',
        'ñ' => 'n',
        _ => return None,
    })
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum Script {
    Latin,
    Cyrillic,
    Greek,
    Other,
}

fn script_of(c: char) -> Option<Script> {
    if c.is_ascii_digit() || c == '-' {
        return None;
    }
    Some(match c as u32 {
        0x41..=0x5A | 0x61..=0x7A | 0xC0..=0x24F | 0x1E00..=0x1EFF => Script::Latin,
        0x370..=0x3FF | 0x1F00..=0x1FFF => Script::Greek,
        0x400..=0x52F => Script::Cyrillic,
        _ => Script::Other,
    })
}

fn brand_of(token: &str) -> Option<(&'static str, &'static [&'static str])> {
    BRANDS
        .iter()
        .find(|(key, _, _)| *key == token)
        .map(|(_, name, domains)| (*name, *domains))
}

fn owned_by(host: &str, domains: &[&str]) -> bool {
    if domains
        .iter()
        .any(|d| host == *d || host.ends_with(&format!(".{d}")))
    {
        return true;
    }
    // The brand's own name on a country domain: google.com.hk, amazon.co.jp,
    // apple.com.cn. Only country suffixes, so paypal.xyz is not the brand's.
    let Some(registrable) = psl::domain_str(host) else {
        return false;
    };
    let suffix = psl::suffix_str(host).unwrap_or("");
    let country = suffix.rsplit('.').next().is_some_and(|tld| tld.len() == 2);
    let name = registrable.split('.').next().unwrap_or("");
    country
        && domains
            .iter()
            .any(|d| d.split('.').next() == Some(name) && d.ends_with(".com"))
}

/// Whether a page address imitates a well-known site. Works on the address
/// only; the brands' own domains never match.
pub fn lookalike_for_url(url: &str) -> Option<Lookalike> {
    let host = host_of(url)?;
    if host.starts_with('[') || host.parse::<std::net::Ipv4Addr>().is_ok() {
        return None;
    }
    let (unicode, _) = idna::domain_to_unicode(&host);
    let registrable = psl::domain_str(&host).unwrap_or(&host).to_string();
    let suffix = psl::suffix_str(&host).unwrap_or("");
    let name = registrable
        .strip_suffix(suffix)
        .map(|n| n.trim_end_matches('.'))
        .unwrap_or(&registrable);
    let name_unicode = idna::domain_to_unicode(name).0;

    // 1. Look-alike letters: decoded, the registrable name reads as a brand.
    if !name_unicode.is_ascii() {
        let twin: String = name_unicode
            .chars()
            .map(|c| latin_twin(c).unwrap_or(c))
            .collect();
        for token in twin.split(['-', '.']) {
            if let Some((brand, domains)) = brand_of(token) {
                if !owned_by(&host, domains) {
                    return Some(Lookalike {
                        brand,
                        reason: "homograph",
                    });
                }
            }
        }
        // Latin mixed with Cyrillic or Greek inside one name (Chrome shows
        // such names as punycode for the same reason).
        for label in unicode.split('.') {
            let scripts: Vec<Script> = label.chars().filter_map(script_of).collect();
            let latin = scripts.contains(&Script::Latin);
            let other = scripts
                .iter()
                .any(|s| matches!(s, Script::Cyrillic | Script::Greek));
            if latin && other {
                return Some(Lookalike {
                    brand: "a well-known site",
                    reason: "mixed-script",
                });
            }
        }
    }

    // 2. A brand's domain used as a subdomain: paypal.com.verify.example.top
    for (_, brand, domains) in BRANDS {
        if owned_by(&host, domains) {
            continue;
        }
        for domain in *domains {
            if host.starts_with(&format!("{domain}.")) || host.contains(&format!(".{domain}.")) {
                return Some(Lookalike {
                    brand,
                    reason: "brand-subdomain",
                });
            }
        }
    }

    // 3. A brand name next to a lure word, on a domain the brand does not own:
    // paypal-login-verify.com, secure-apple-id.top, appleid.verify-login.xyz
    let labels: Vec<&str> = host
        .strip_suffix(suffix)
        .unwrap_or(&host)
        .trim_end_matches('.')
        .split('.')
        .collect();
    let tokens: Vec<&str> = labels.iter().flat_map(|l| l.split('-')).collect();
    for token in &tokens {
        // The brand alone, or glued to a lure (paypallogin, secureapple, appleid).
        let found = brand_of(token)
            .map(|(brand, domains)| (brand, domains, ""))
            .or_else(|| {
                BRANDS.iter().find_map(|(key, brand, domains)| {
                    let rest = token
                        .strip_prefix(key)
                        .or_else(|| token.strip_suffix(key))?;
                    (rest == "id" || LURES.contains(&rest)).then_some((*brand, *domains, rest))
                })
            });
        let Some((brand, domains, glued)) = found else {
            continue;
        };
        if owned_by(&host, domains) {
            continue;
        }
        let lured = (!glued.is_empty() && glued != "id")
            || tokens.iter().any(|t| t != token && LURES.contains(t));
        if lured {
            return Some(Lookalike {
                brand,
                reason: "brand-lure",
            });
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn first(kind: &str) -> &'static str {
        THREATS
            .lines()
            .find_map(|l| l.strip_prefix(kind).map(str::trim))
            .filter(|h| !h.starts_with('['))
            .expect("the table has an entry of this kind")
    }

    #[test]
    fn listed_sites_and_their_subdomains_are_dangerous() {
        let malware = first("m ");
        let phishing = first("p ");
        assert_eq!(
            threat_for_url(&format!("http://{malware}/x")),
            Some(Threat::Malware)
        );
        assert_eq!(
            threat_for_url(&format!("https://{phishing}/login?u=1")),
            Some(Threat::Phishing)
        );
        assert_eq!(
            threat_for_url(&format!("https://deep.sub.{phishing}/")),
            Some(Threat::Phishing)
        );
        assert_eq!(
            threat_for_url(&format!("https://user:pw@{phishing}:8443/")),
            Some(Threat::Phishing)
        );
        assert_eq!(
            threat_for_url(&format!("https://{}./", phishing.to_uppercase())),
            Some(Threat::Phishing)
        );
    }

    #[test]
    fn ordinary_sites_are_not() {
        for url in [
            "https://www.google.com/",
            "https://github.com/Moha2005269/agentgo",
            "https://example.com/",
            "https://en.wikipedia.org/wiki/Phishing",
            "agentgo://shell/index.html",
            "about:blank",
        ] {
            assert_eq!(threat_for_url(url), None, "{url}");
        }
        assert!(threat_count() > 1000);
    }

    #[test]
    fn a_parent_of_a_listed_host_is_not_listed() {
        // Free hosting: one bad page on a subdomain must not block the host.
        let sub = THREATS
            .lines()
            .filter_map(|l| l.split_once(' ').map(|(_, h)| h))
            .find(|h| h.ends_with(".weebly.com") || h.ends_with(".replit.app"));
        if let Some(sub) = sub {
            let parent = sub.split_once('.').map(|(_, p)| p).unwrap_or(sub);
            if threat_for_url(&format!("https://{parent}/")).is_none() {
                assert!(threat_for_url(&format!("https://{sub}/")).is_some());
            }
        }
    }

    #[test]
    fn look_alike_sites_are_recognised() {
        let flagged = |url: &str| lookalike_for_url(url).map(|l| (l.brand, l.reason));
        // Cyrillic "а" in paypal, encoded as the browser sees it.
        let cyrillic = format!(
            "https://{}/",
            idna::domain_to_ascii("p\u{430}ypal.com").unwrap()
        );
        assert_eq!(flagged(&cyrillic), Some(("PayPal", "homograph")));
        assert_eq!(
            flagged("https://paypal.com.account-verify.top/login"),
            Some(("PayPal", "brand-subdomain"))
        );
        assert_eq!(
            flagged("https://www.microsoftonline.com.sign-in.example.net/"),
            Some(("Microsoft", "brand-subdomain"))
        );
        assert_eq!(
            flagged("https://paypal-login-secure.com/"),
            Some(("PayPal", "brand-lure"))
        );
        assert_eq!(
            flagged("https://secure-appleid.top/"),
            Some(("Apple", "brand-lure"))
        );
        assert_eq!(
            flagged("https://paypallogin.xyz/"),
            Some(("PayPal", "brand-lure"))
        );
        assert_eq!(
            flagged("https://netflix.billing-update.info/"),
            Some(("Netflix", "brand-lure"))
        );
        assert_eq!(
            flagged("https://paypal.com.mx.secure-login.top/"),
            Some(("PayPal", "brand-subdomain"))
        );
    }

    #[test]
    fn real_and_unrelated_sites_are_not() {
        for url in [
            "https://www.paypal.com/signin",
            "https://login.microsoftonline.com/common/oauth2",
            "https://accounts.google.com/v3/signin",
            "https://appleid.apple.com/",
            "https://secure.chase.com/",
            "https://www.amazon.co.uk/ap/signin",
            "https://steamcommunity.com/login",
            "https://github.com/login",
            "https://www.applebees.com/",
            "https://pineapple-recipes.com/",
            "https://purchase-orders.example.com/login",
            "https://münchen.de/",
            "https://example.com/paypal-login",
            "https://192.168.1.1/login",
            "https://www.google.com.hk/",
            "https://www.amazon.com.mx/ap/signin",
            "https://www.apple.com.cn/",
            "https://amazonaws.com.cn/",
        ] {
            assert_eq!(lookalike_for_url(url), None, "{url}");
        }
    }

    /// False-positive survey over popular sites (one domain per line):
    /// `AGENTGO_DOMAINS=top100k.txt cargo test -p agentgo-adblock -- --ignored survey --nocapture`
    #[test]
    #[ignore]
    fn survey_popular_sites() {
        let Some(file) = std::env::var_os("AGENTGO_DOMAINS") else {
            return;
        };
        let text = std::fs::read_to_string(file).unwrap();
        let (mut listed, mut alike, mut total) = (0, 0, 0);
        for domain in text.lines().map(str::trim).filter(|d| !d.is_empty()) {
            total += 1;
            let url = format!("https://{domain}/");
            if let Some(threat) = threat_for_url(&url) {
                listed += 1;
                println!("LISTED {} {domain}", threat.code());
            }
            if let Some(hit) = lookalike_for_url(&url) {
                alike += 1;
                println!("ALIKE {} {} {domain}", hit.reason, hit.brand);
            }
        }
        println!("survey: {total} domains, {listed} listed, {alike} look-alike");
    }
}
