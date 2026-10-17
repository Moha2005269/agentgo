//! Firefox-specific readers: NSS-protected logins (`key4.db` + `logins.json`)
//! and compressed session-store tabs (`*.jsonlz4`).
//!
//! Only profiles without a primary password are supported; with one set, the
//! key cannot be derived and the caller reports that instead.

use super::{copied_database, valid_web_url, Login};
use aes::Aes256;
use cbc::cipher::block_padding::Pkcs7;
use cbc::cipher::{BlockDecryptMut, KeyIvInit};
use des::TdesEde3;
use ring::{digest, hmac, pbkdf2};
use serde_json::Value;
use std::fs;
use std::num::NonZeroU32;
use std::path::Path;

const OID_PBES2: &[u8] = &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x05, 0x0D];
const OID_AES256_CBC: &[u8] = &[0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x01, 0x2A];
const OID_DES3_CBC: &[u8] = &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x03, 0x07];
const OID_PBE_SHA1_3DES: &[u8] = &[
    0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x0C, 0x05, 0x01, 0x03,
];

const MAX_SESSION_BYTES: usize = 64 * 1024 * 1024;

/// Outcome of reading Firefox logins.
pub(crate) enum LoginsResult {
    Logins(Vec<Login>),
    NoLogins,
    PrimaryPassword,
    Unreadable,
}

pub(crate) fn read_logins(profile: &Path) -> LoginsResult {
    let Some(json) = fs::read(profile.join("logins.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
    else {
        return LoginsResult::NoLogins;
    };
    let Some(entries) = json.get("logins").and_then(Value::as_array) else {
        return LoginsResult::NoLogins;
    };
    if entries.is_empty() {
        return LoginsResult::NoLogins;
    }
    let key = match load_key(profile) {
        KeyResult::Key(key) => key,
        KeyResult::PrimaryPassword => return LoginsResult::PrimaryPassword,
        KeyResult::Missing => return LoginsResult::Unreadable,
    };
    let mut logins = Vec::new();
    for entry in entries {
        let origin = entry
            .get("hostname")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if !valid_web_url(origin) {
            continue;
        }
        let field = |name: &str| {
            entry
                .get(name)
                .and_then(Value::as_str)
                .and_then(super::decode_base64)
                .and_then(|bytes| decrypt_login_field(&key, &bytes))
        };
        if let (Some(username), Some(password)) =
            (field("encryptedUsername"), field("encryptedPassword"))
        {
            if !username.trim().is_empty() && !password.is_empty() {
                logins.push(Login {
                    origin: origin.to_owned(),
                    username,
                    password,
                });
            }
        }
    }
    if logins.is_empty() {
        LoginsResult::Unreadable
    } else {
        LoginsResult::Logins(logins)
    }
}

enum KeyResult {
    Key(Vec<u8>),
    PrimaryPassword,
    Missing,
}

fn load_key(profile: &Path) -> KeyResult {
    let Ok(Some(conn)) = copied_database(&profile.join("key4.db")) else {
        return KeyResult::Missing;
    };
    let meta: Option<(Vec<u8>, Vec<u8>)> = conn
        .query_row(
            "SELECT item1, item2 FROM metaData WHERE id = 'password'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .ok();
    let Some((global_salt, check)) = meta else {
        return KeyResult::Missing;
    };
    match decrypt_pbe(&check, &global_salt, b"") {
        Some(plain) if plain.starts_with(b"password-check") => {}
        _ => return KeyResult::PrimaryPassword,
    }
    let Ok(mut stmt) = conn.prepare("SELECT a11 FROM nssPrivate WHERE a11 IS NOT NULL") else {
        return KeyResult::Missing;
    };
    let Ok(rows) = stmt.query_map([], |row| row.get::<_, Vec<u8>>(0)) else {
        return KeyResult::Missing;
    };
    for blob in rows.flatten() {
        if let Some(key) = decrypt_pbe(&blob, &global_salt, b"") {
            if key.len() >= 24 {
                return KeyResult::Key(key);
            }
        }
    }
    KeyResult::Missing
}

struct Tlv<'a> {
    body: &'a [u8],
}

fn read_tlv(data: &[u8]) -> Option<(Tlv<'_>, &[u8])> {
    data.first()?;
    let first = *data.get(1)?;
    let (len, header) = if first < 0x80 {
        (first as usize, 2)
    } else {
        let n = (first & 0x7F) as usize;
        if n == 0 || n > 4 {
            return None;
        }
        let mut len = 0usize;
        for byte in data.get(2..2 + n)? {
            len = (len << 8) | *byte as usize;
        }
        (len, 2 + n)
    };
    let end = header.checked_add(len)?;
    let body = data.get(header..end)?;
    Some((Tlv { body }, &data[end..]))
}

fn der_int(bytes: &[u8]) -> Option<u32> {
    if bytes.is_empty() || bytes.len() > 4 {
        return None;
    }
    Some(bytes.iter().fold(0u32, |acc, b| (acc << 8) | *b as u32))
}

fn sha1(parts: &[&[u8]]) -> Vec<u8> {
    let mut ctx = digest::Context::new(&digest::SHA1_FOR_LEGACY_USE_ONLY);
    for part in parts {
        ctx.update(part);
    }
    ctx.finish().as_ref().to_vec()
}

fn hmac_sha1(key: &[u8], parts: &[&[u8]]) -> Vec<u8> {
    let key = hmac::Key::new(hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY, key);
    let mut ctx = hmac::Context::with_key(&key);
    for part in parts {
        ctx.update(part);
    }
    ctx.sign().as_ref().to_vec()
}

fn aes256_cbc_decrypt(key: &[u8], iv: &[u8], data: &[u8]) -> Option<Vec<u8>> {
    let mut buf = data.to_vec();
    let plain = cbc::Decryptor::<Aes256>::new_from_slices(key, iv)
        .ok()?
        .decrypt_padded_mut::<Pkcs7>(&mut buf)
        .ok()?
        .to_vec();
    Some(plain)
}

fn des3_cbc_decrypt(key: &[u8], iv: &[u8], data: &[u8]) -> Option<Vec<u8>> {
    let mut buf = data.to_vec();
    let plain = cbc::Decryptor::<TdesEde3>::new_from_slices(key, iv)
        .ok()?
        .decrypt_padded_mut::<Pkcs7>(&mut buf)
        .ok()?
        .to_vec();
    Some(plain)
}

/// Decrypt an NSS `EncryptedData` structure protected by the primary
/// password (empty here) and the global salt.
fn decrypt_pbe(der: &[u8], global_salt: &[u8], password: &[u8]) -> Option<Vec<u8>> {
    let (outer, _) = read_tlv(der)?;
    let (algorithm, rest) = read_tlv(outer.body)?;
    let (cipher_text, _) = read_tlv(rest)?;
    let (oid, params) = read_tlv(algorithm.body)?;
    let (params, _) = read_tlv(params)?;
    if oid.body == OID_PBES2 {
        let (kdf, rest) = read_tlv(params.body)?;
        let (enc, _) = read_tlv(rest)?;
        let (_kdf_oid, kdf_params) = read_tlv(kdf.body)?;
        let (kdf_params, _) = read_tlv(kdf_params)?;
        let (salt, rest) = read_tlv(kdf_params.body)?;
        let (iterations, _) = read_tlv(rest)?;
        let iterations = NonZeroU32::new(der_int(iterations.body)?)?;
        let (_enc_oid, iv) = read_tlv(enc.body)?;
        let (iv, _) = read_tlv(iv)?;
        let mut full_iv = Vec::new();
        if iv.body.len() == 14 {
            full_iv.extend_from_slice(&[0x04, 0x0E]);
        }
        full_iv.extend_from_slice(iv.body);
        let secret = sha1(&[global_salt, password]);
        let mut key = [0u8; 32];
        pbkdf2::derive(
            pbkdf2::PBKDF2_HMAC_SHA256,
            iterations,
            salt.body,
            &secret,
            &mut key,
        );
        return aes256_cbc_decrypt(&key, &full_iv, cipher_text.body);
    }
    if oid.body == OID_PBE_SHA1_3DES {
        let (salt, _) = read_tlv(params.body)?;
        let entry_salt = salt.body;
        let hp = sha1(&[global_salt, password]);
        let mut pes = entry_salt.to_vec();
        pes.resize(pes.len().max(20), 0);
        let chp = sha1(&[&hp, entry_salt]);
        let k1 = hmac_sha1(&chp, &[&pes, entry_salt]);
        let tk = hmac_sha1(&chp, &[&pes]);
        let k2 = hmac_sha1(&chp, &[&tk, entry_salt]);
        let mut k = k1;
        k.extend_from_slice(&k2);
        return des3_cbc_decrypt(&k[..24], &k[k.len() - 8..], cipher_text.body);
    }
    None
}

fn decrypt_login_field(key: &[u8], der: &[u8]) -> Option<String> {
    let (outer, _) = read_tlv(der)?;
    let (_key_id, rest) = read_tlv(outer.body)?;
    let (algorithm, rest) = read_tlv(rest)?;
    let (cipher_text, _) = read_tlv(rest)?;
    let (oid, iv) = read_tlv(algorithm.body)?;
    let (iv, _) = read_tlv(iv)?;
    let plain = if oid.body == OID_AES256_CBC {
        aes256_cbc_decrypt(key.get(..32)?, iv.body, cipher_text.body)?
    } else if oid.body == OID_DES3_CBC {
        des3_cbc_decrypt(key.get(..24)?, iv.body, cipher_text.body)?
    } else {
        return None;
    };
    String::from_utf8(plain).ok()
}

/// Decode a Mozilla `mozLz40` file (magic, little-endian size, LZ4 block).
pub(crate) fn decode_mozlz4(bytes: &[u8]) -> Option<Vec<u8>> {
    let rest = bytes.strip_prefix(b"mozLz40\0")?;
    let size = u32::from_le_bytes(rest.get(..4)?.try_into().ok()?) as usize;
    if size > MAX_SESSION_BYTES {
        return None;
    }
    lz4_block(&rest[4..], size)
}

fn lz4_block(input: &[u8], expected: usize) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(expected);
    let mut i = 0usize;
    let extended = |input: &[u8], i: &mut usize, mut len: usize| -> Option<usize> {
        if len == 15 {
            loop {
                let byte = *input.get(*i)?;
                *i += 1;
                len = len.checked_add(byte as usize)?;
                if byte != 255 {
                    break;
                }
            }
        }
        Some(len)
    };
    while i < input.len() {
        let token = input[i];
        i += 1;
        let literals = extended(input, &mut i, (token >> 4) as usize)?;
        out.extend_from_slice(input.get(i..i.checked_add(literals)?)?);
        i += literals;
        if out.len() > expected {
            return None;
        }
        if i >= input.len() {
            break;
        }
        let offset = u16::from_le_bytes(input.get(i..i + 2)?.try_into().ok()?) as usize;
        i += 2;
        if offset == 0 || offset > out.len() {
            return None;
        }
        let length = extended(input, &mut i, (token & 15) as usize)?.checked_add(4)?;
        if out.len().checked_add(length)? > expected {
            return None;
        }
        let start = out.len() - offset;
        for n in 0..length {
            let byte = out[start + n];
            out.push(byte);
        }
    }
    (out.len() == expected).then_some(out)
}

/// Open-tab URLs from Firefox's session store, active entry of each tab.
pub(crate) fn session_tabs(profile: &Path) -> Vec<String> {
    for relative in [
        "sessionstore-backups/recovery.jsonlz4",
        "sessionstore-backups/previous.jsonlz4",
        "sessionstore.jsonlz4",
    ] {
        let Some(bytes) = fs::read(profile.join(relative)).ok() else {
            continue;
        };
        let Some(json) =
            decode_mozlz4(&bytes).and_then(|plain| serde_json::from_slice::<Value>(&plain).ok())
        else {
            continue;
        };
        let tabs = tabs_from_session(&json);
        if !tabs.is_empty() {
            return tabs;
        }
    }
    Vec::new()
}

fn tabs_from_session(json: &Value) -> Vec<String> {
    let mut urls: Vec<String> = Vec::new();
    let windows = json
        .get("windows")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    for window in windows {
        for tab in window
            .get("tabs")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default()
        {
            let Some(entries) = tab.get("entries").and_then(Value::as_array) else {
                continue;
            };
            let index = tab
                .get("index")
                .and_then(Value::as_u64)
                .unwrap_or(entries.len() as u64)
                .clamp(1, entries.len().max(1) as u64) as usize;
            let url = entries
                .get(index - 1)
                .and_then(|entry| entry.get("url"))
                .and_then(Value::as_str);
            if let Some(url) = url.filter(|url| valid_web_url(url)) {
                if !urls.iter().any(|existing| existing == url) {
                    urls.push(url.to_owned());
                }
            }
        }
    }
    urls
}

#[cfg(test)]
mod tests {
    use super::*;
    use cbc::cipher::{block_padding::Pkcs7, BlockEncryptMut};

    fn tlv(tag: u8, body: &[u8]) -> Vec<u8> {
        let mut out = vec![tag];
        if body.len() < 128 {
            out.push(body.len() as u8);
        } else {
            out.extend_from_slice(&[0x82, (body.len() >> 8) as u8, body.len() as u8]);
        }
        out.extend_from_slice(body);
        out
    }

    fn seq(parts: &[Vec<u8>]) -> Vec<u8> {
        tlv(0x30, &parts.concat())
    }

    fn aes_encrypt(key: &[u8], iv: &[u8], plain: &[u8]) -> Vec<u8> {
        let mut buf = vec![0u8; plain.len() + 16];
        buf[..plain.len()].copy_from_slice(plain);
        cbc::Encryptor::<Aes256>::new_from_slices(key, iv)
            .unwrap()
            .encrypt_padded_mut::<Pkcs7>(&mut buf, plain.len())
            .unwrap()
            .to_vec()
    }

    fn des_encrypt(key: &[u8], iv: &[u8], plain: &[u8]) -> Vec<u8> {
        let mut buf = vec![0u8; plain.len() + 8];
        buf[..plain.len()].copy_from_slice(plain);
        cbc::Encryptor::<TdesEde3>::new_from_slices(key, iv)
            .unwrap()
            .encrypt_padded_mut::<Pkcs7>(&mut buf, plain.len())
            .unwrap()
            .to_vec()
    }

    fn pbes2(global_salt: &[u8], password: &[u8], plain: &[u8]) -> Vec<u8> {
        let salt = [7u8; 32];
        let iterations = 10u32;
        let secret = sha1(&[global_salt, password]);
        let mut key = [0u8; 32];
        pbkdf2::derive(
            pbkdf2::PBKDF2_HMAC_SHA256,
            NonZeroU32::new(iterations).unwrap(),
            &salt,
            &secret,
            &mut key,
        );
        let iv14 = [9u8; 14];
        let mut iv = vec![0x04, 0x0E];
        iv.extend_from_slice(&iv14);
        let cipher = aes_encrypt(&key, &iv, plain);
        seq(&[
            seq(&[
                tlv(0x06, OID_PBES2),
                seq(&[
                    seq(&[
                        tlv(
                            0x06,
                            &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x05, 0x0C],
                        ),
                        seq(&[
                            tlv(0x04, &salt),
                            tlv(0x02, &[iterations as u8]),
                            tlv(0x02, &[32]),
                            seq(&[tlv(0x06, &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x02, 0x09])]),
                        ]),
                    ]),
                    seq(&[tlv(0x06, OID_AES256_CBC), tlv(0x04, &iv14)]),
                ]),
            ]),
            tlv(0x04, &cipher),
        ])
    }

    fn legacy(global_salt: &[u8], plain: &[u8]) -> Vec<u8> {
        let entry_salt = [3u8; 20];
        let hp = sha1(&[global_salt, b""]);
        let chp = sha1(&[&hp, &entry_salt]);
        let k1 = hmac_sha1(&chp, &[&entry_salt, &entry_salt]);
        let tk = hmac_sha1(&chp, &[&entry_salt]);
        let k2 = hmac_sha1(&chp, &[&tk, &entry_salt]);
        let mut k = k1;
        k.extend_from_slice(&k2);
        let cipher = des_encrypt(&k[..24], &k[k.len() - 8..], plain);
        seq(&[
            seq(&[
                tlv(0x06, OID_PBE_SHA1_3DES),
                seq(&[tlv(0x04, &entry_salt), tlv(0x02, &[1])]),
            ]),
            tlv(0x04, &cipher),
        ])
    }

    fn login_field(key: &[u8], aes: bool, text: &str) -> String {
        let (oid, iv, cipher) = if aes {
            let iv = [5u8; 16];
            (
                OID_AES256_CBC,
                iv.to_vec(),
                aes_encrypt(&key[..32], &iv, text.as_bytes()),
            )
        } else {
            let iv = [6u8; 8];
            (
                OID_DES3_CBC,
                iv.to_vec(),
                des_encrypt(&key[..24], &iv, text.as_bytes()),
            )
        };
        let der = seq(&[
            tlv(0x04, &[0xF8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]),
            seq(&[tlv(0x06, oid), tlv(0x04, &iv)]),
            tlv(0x04, &cipher),
        ]);
        base64(&der)
    }

    fn base64(data: &[u8]) -> String {
        const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in data.chunks(3) {
            let n = (chunk[0] as u32) << 16
                | (*chunk.get(1).unwrap_or(&0) as u32) << 8
                | *chunk.get(2).unwrap_or(&0) as u32;
            for i in 0..4 {
                if i <= chunk.len() {
                    out.push(T[((n >> (18 - 6 * i)) & 63) as usize] as char);
                } else {
                    out.push('=');
                }
            }
        }
        out
    }

    fn profile(label: &str, modern: bool, primary_password: bool) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("agentgo-ff-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let global_salt = [1u8; 20];
        let master: Vec<u8> = (0..32).collect();
        let (check, key_blob) = if modern {
            let pw: &[u8] = if primary_password { b"secret" } else { b"" };
            (
                pbes2(&global_salt, pw, b"password-check\x02\x02"),
                pbes2(&global_salt, pw, &master),
            )
        } else {
            (
                legacy(&global_salt, b"password-check\x02\x02"),
                legacy(&global_salt, &master[..24]),
            )
        };
        let conn = rusqlite::Connection::open(dir.join("key4.db")).unwrap();
        conn.execute_batch(
            "CREATE TABLE metaData (id TEXT PRIMARY KEY, item1, item2);
             CREATE TABLE nssPrivate (a11, a102);",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO metaData VALUES ('password', ?1, ?2)",
            rusqlite::params![&global_salt[..], &check],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO nssPrivate VALUES (?1, x'f8')",
            rusqlite::params![&key_blob],
        )
        .unwrap();
        drop(conn);
        let logins = serde_json::json!({"logins": [
            {"hostname": "https://example.com",
             "encryptedUsername": login_field(&master, modern, "alice@example.com"),
             "encryptedPassword": login_field(&master, modern, "p@ss w0rd ü")},
            {"hostname": "chrome://sync",
             "encryptedUsername": login_field(&master, modern, "skip"),
             "encryptedPassword": login_field(&master, modern, "skip")}
        ]});
        fs::write(dir.join("logins.json"), logins.to_string()).unwrap();
        dir
    }

    #[test]
    fn decrypts_modern_pbes2_logins() {
        let dir = profile("modern", true, false);
        let LoginsResult::Logins(logins) = read_logins(&dir) else {
            panic!("expected logins");
        };
        assert_eq!(logins.len(), 1);
        assert_eq!(logins[0].username, "alice@example.com");
        assert_eq!(logins[0].password, "p@ss w0rd ü");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn decrypts_legacy_3des_logins() {
        let dir = profile("legacy", false, false);
        let LoginsResult::Logins(logins) = read_logins(&dir) else {
            panic!("expected logins");
        };
        assert_eq!(logins[0].password, "p@ss w0rd ü");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn reports_primary_password() {
        let dir = profile("primary", true, true);
        assert!(matches!(read_logins(&dir), LoginsResult::PrimaryPassword));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn lz4_roundtrip_with_matches() {
        // "abcabcabcabc" + "tail": literal "abc", match offset 3 len 9, literal "tail".
        let mut block = vec![0x35, b'a', b'b', b'c', 3, 0];
        block.push(0x40);
        block.extend_from_slice(b"tail");
        let plain = lz4_block(&block, 16).unwrap();
        assert_eq!(plain, b"abcabcabcabctail");
        assert!(lz4_block(&block, 15).is_none());
        assert!(lz4_block(&[0x10, b'a', 9, 0], 8).is_none());
    }

    #[test]
    fn session_tabs_use_active_entry() {
        let json = serde_json::json!({"windows": [{"tabs": [
            {"index": 2, "entries": [{"url": "https://a.test/1"}, {"url": "https://a.test/2"}]},
            {"entries": [{"url": "about:blank"}, {"url": "https://b.test/"}]},
            {"index": 1, "entries": [{"url": "about:newtab"}]}
        ]}]});
        assert_eq!(
            tabs_from_session(&json),
            vec!["https://a.test/2".to_owned(), "https://b.test/".to_owned()]
        );
    }

    #[test]
    fn mozlz4_header_is_required() {
        assert!(decode_mozlz4(b"nope").is_none());
        let mut file = b"mozLz40\0".to_vec();
        file.extend_from_slice(&3u32.to_le_bytes());
        file.extend_from_slice(&[0x30, b'x', b'y', b'z']);
        assert_eq!(decode_mozlz4(&file).unwrap(), b"xyz");
    }
}
