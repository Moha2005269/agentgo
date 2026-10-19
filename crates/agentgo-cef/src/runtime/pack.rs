//! Package handling that does not trust its input: CRX signature checks and
//! a zip extractor that cannot write outside its destination.

use std::io::Read;
use std::path::Path;

use ring::digest;
use ring::signature::{self, UnparsedPublicKey};

const MAX_ENTRIES: usize = 20_000;
const MAX_TOTAL: u64 = 4 << 30;

/// Extracts `bytes` (a zip) into `dest`. Entries with `..`, absolute or
/// drive-letter paths are refused; entry and total sizes are bounded.
pub fn extract_zip(bytes: &[u8], dest: &Path) -> Result<usize, String> {
    let mut archive =
        zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
    if archive.len() > MAX_ENTRIES {
        return Err("the archive has too many entries".into());
    }
    std::fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    let root = dest.canonicalize().map_err(|e| e.to_string())?;
    let mut total = 0u64;
    let mut files = 0usize;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|e| e.to_string())?;
        let Some(relative) = entry.enclosed_name() else {
            return Err(format!("unsafe path in archive: {}", entry.name()));
        };
        if entry.is_symlink() {
            return Err(format!("the archive contains a link: {}", entry.name()));
        }
        let target = root.join(relative);
        if entry.is_dir() {
            std::fs::create_dir_all(&target).map_err(|e| e.to_string())?;
            continue;
        }
        total += entry.size();
        if total > MAX_TOTAL {
            return Err("the archive unpacks to more than 1 GB".into());
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut out = std::fs::File::create(&target).map_err(|e| e.to_string())?;
        // `take` guards against entries that lie about their size.
        std::io::copy(&mut (&mut entry).take(MAX_TOTAL), &mut out).map_err(|e| e.to_string())?;
        files += 1;
    }
    Ok(files)
}

fn varint(data: &[u8], at: &mut usize) -> Option<u64> {
    let mut value = 0u64;
    for shift in (0..64).step_by(7) {
        let byte = *data.get(*at)?;
        *at += 1;
        value |= u64::from(byte & 0x7F) << shift;
        if byte & 0x80 == 0 {
            return Some(value);
        }
    }
    None
}

/// Walks a protobuf message, returning `(field, bytes)` for length-delimited fields.
fn fields(data: &[u8]) -> Option<Vec<(u64, &[u8])>> {
    let mut at = 0;
    let mut out = Vec::new();
    while at < data.len() {
        let key = varint(data, &mut at)?;
        match key & 7 {
            0 => {
                varint(data, &mut at)?;
            }
            2 => {
                let len = varint(data, &mut at)? as usize;
                out.push((key >> 3, data.get(at..at.checked_add(len)?)?));
                at += len;
            }
            1 => at = at.checked_add(8)?,
            5 => at = at.checked_add(4)?,
            _ => return None,
        }
    }
    Some(out)
}

/// A DER element: `(tag, content, rest)`.
fn der(data: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    let tag = *data.first()?;
    let first = *data.get(1)?;
    let (len, header) = if first < 0x80 {
        (first as usize, 2)
    } else {
        let count = (first & 0x7F) as usize;
        if count == 0 || count > 4 {
            return None;
        }
        let mut len = 0usize;
        for byte in data.get(2..2 + count)? {
            len = (len << 8) | *byte as usize;
        }
        (len, 2 + count)
    };
    let end = header.checked_add(len)?;
    Some((tag, data.get(header..end)?, data.get(end..)?))
}

enum Key<'a> {
    Rsa(&'a [u8]),
    P256(&'a [u8]),
}

/// The raw key inside a SubjectPublicKeyInfo: PKCS#1 for RSA, the point for P-256.
fn spki_key(spki: &[u8]) -> Option<Key<'_>> {
    let (_, outer, _) = der(spki)?;
    let (_, algorithm, rest) = der(outer)?;
    let (_, bits, _) = der(rest)?;
    let key = bits.get(1..)?;
    let (_, oid, _) = der(algorithm)?;
    // 1.2.840.113549.1.1.1 (rsaEncryption) or 1.2.840.10045.2.1 (ecPublicKey)
    match oid {
        [0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x01, 0x01] => Some(Key::Rsa(key)),
        [0x2A, 0x86, 0x48, 0xCE, 0x3D, 0x02, 0x01] => Some(Key::P256(key)),
        _ => None,
    }
}

/// Chrome's extension id for a public key: 16 bytes of its SHA-256, a..p.
pub fn extension_id(spki: &[u8]) -> String {
    digest::digest(&digest::SHA256, spki).as_ref()[..16]
        .iter()
        .flat_map(|b| [b >> 4, b & 15])
        .map(|n| (b'a' + n) as char)
        .collect()
}

/// Checks a CRX package's signature and that it belongs to `expected_id`.
/// Returns the zip archive inside it.
#[cfg(test)]
pub fn verify_crx<'a>(bytes: &'a [u8], expected_id: &str) -> Result<&'a [u8], String> {
    verify_crx_with_key(bytes, expected_id).map(|(zip, _)| zip)
}

/// [`verify_crx`], also returning the public key (SubjectPublicKeyInfo) the
/// package is signed with: written into the unpacked manifest as `key`, it
/// gives the extension its Web Store id when Chromium loads the folder.
pub fn verify_crx_with_key<'a>(
    bytes: &'a [u8],
    expected_id: &str,
) -> Result<(&'a [u8], &'a [u8]), String> {
    let word = |at: usize| {
        bytes
            .get(at..at + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
    };
    if bytes.get(0..4) != Some(b"Cr24") {
        return Err("not an extension package".into());
    }
    match word(4).ok_or("truncated package")? {
        3 => {
            let header_len = word(8).ok_or("truncated package")?;
            let header = bytes.get(12..12 + header_len).ok_or("truncated package")?;
            let zip = bytes.get(12 + header_len..).ok_or("truncated package")?;
            let parts = fields(header).ok_or("malformed package header")?;
            let signed_data = parts
                .iter()
                .find(|(f, _)| *f == 10000)
                .map(|(_, v)| *v)
                .ok_or("package has no signed header")?;
            let crx_id = fields(signed_data)
                .and_then(|f| f.into_iter().find(|(n, _)| *n == 1).map(|(_, v)| v))
                .ok_or("package has no id")?;
            let mut message = b"CRX3 SignedData\0".to_vec();
            message.extend((signed_data.len() as u32).to_le_bytes());
            message.extend(signed_data);
            message.extend(zip);
            for (field, proof) in &parts {
                if *field != 2 && *field != 3 {
                    continue;
                }
                let proof = fields(proof).ok_or("malformed key proof")?;
                let key = proof.iter().find(|(n, _)| *n == 1).map(|(_, v)| *v);
                let sig = proof.iter().find(|(n, _)| *n == 2).map(|(_, v)| *v);
                let (Some(key), Some(sig)) = (key, sig) else {
                    continue;
                };
                let id = digest::digest(&digest::SHA256, key);
                if id.as_ref()[..16] != *crx_id || extension_id(key) != expected_id {
                    continue;
                }
                let ok = match spki_key(key) {
                    // Extensions published long ago (Google Translate among
                    // them) are signed with 1024-bit keys, which Chrome accepts.
                    Some(Key::Rsa(k)) if *field == 2 => {
                        UnparsedPublicKey::new(&signature::RSA_PKCS1_2048_8192_SHA256, k)
                            .verify(&message, sig)
                            .is_ok()
                            || UnparsedPublicKey::new(
                                &signature::RSA_PKCS1_1024_8192_SHA256_FOR_LEGACY_USE_ONLY,
                                k,
                            )
                            .verify(&message, sig)
                            .is_ok()
                    }
                    Some(Key::P256(k)) if *field == 3 => {
                        UnparsedPublicKey::new(&signature::ECDSA_P256_SHA256_ASN1, k)
                            .verify(&message, sig)
                            .is_ok()
                    }
                    _ => false,
                };
                if ok {
                    return Ok((zip, key));
                }
            }
            Err("the package signature is not valid for this extension id".into())
        }
        2 => {
            let (key_len, sig_len) = (word(8).ok_or("truncated")?, word(12).ok_or("truncated")?);
            let key = bytes.get(16..16 + key_len).ok_or("truncated package")?;
            let sig = bytes
                .get(16 + key_len..16 + key_len + sig_len)
                .ok_or("truncated package")?;
            let zip = bytes
                .get(16 + key_len + sig_len..)
                .ok_or("truncated package")?;
            let Some(Key::Rsa(k)) = spki_key(key) else {
                return Err("unsupported key".into());
            };
            if extension_id(key) != expected_id {
                return Err("the package key does not match the extension id".into());
            }
            UnparsedPublicKey::new(&signature::RSA_PKCS1_2048_8192_SHA1_FOR_LEGACY_USE_ONLY, k)
                .verify(zip, sig)
                .or_else(|_| {
                    UnparsedPublicKey::new(
                        &signature::RSA_PKCS1_1024_8192_SHA1_FOR_LEGACY_USE_ONLY,
                        k,
                    )
                    .verify(zip, sig)
                })
                .map_err(|_| "the package signature is not valid".to_string())?;
            Ok((zip, key))
        }
        _ => Err("unsupported package version".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_garbage_and_truncated_packages() {
        assert!(verify_crx(b"PK\x03\x04", "a").is_err());
        assert!(verify_crx(b"Cr24", "a").is_err());
        let mut crx = b"Cr24".to_vec();
        crx.extend(3u32.to_le_bytes());
        crx.extend(0u32.to_le_bytes());
        crx.extend(b"PKzip");
        assert!(verify_crx(&crx, "a").is_err());
    }

    fn pb(field: u64, data: &[u8]) -> Vec<u8> {
        fn put(out: &mut Vec<u8>, mut value: u64) {
            loop {
                let byte = (value & 0x7F) as u8;
                value >>= 7;
                if value == 0 {
                    out.push(byte);
                    break;
                }
                out.push(byte | 0x80);
            }
        }
        let mut out = Vec::new();
        put(&mut out, (field << 3) | 2);
        put(&mut out, data.len() as u64);
        out.extend(data);
        out
    }

    fn signed_crx(tamper: bool) -> (Vec<u8>, String) {
        use ring::signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_ASN1_SIGNING};
        let rng = ring::rand::SystemRandom::new();
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng).unwrap();
        let pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8.as_ref(), &rng)
            .unwrap();
        let mut spki = hex_bytes("3059301306072a8648ce3d020106082a8648ce3d030107034200");
        spki.extend(pair.public_key().as_ref());
        let id = extension_id(&spki);
        let crx_id = digest::digest(&digest::SHA256, &spki).as_ref()[..16].to_vec();
        let signed_data = pb(1, &crx_id);
        let zip = b"PK\x03\x04payload".to_vec();
        let mut message = b"CRX3 SignedData\0".to_vec();
        message.extend((signed_data.len() as u32).to_le_bytes());
        message.extend(&signed_data);
        message.extend(&zip);
        let signature = pair.sign(&rng, &message).unwrap();
        let mut proof = pb(1, &spki);
        proof.extend(pb(2, signature.as_ref()));
        let mut header = pb(3, &proof);
        header.extend(pb(10000, &signed_data));
        let mut crx = b"Cr24".to_vec();
        crx.extend(3u32.to_le_bytes());
        crx.extend((header.len() as u32).to_le_bytes());
        crx.extend(&header);
        crx.extend(&zip);
        if tamper {
            *crx.last_mut().unwrap() ^= 1;
        }
        (crx, id)
    }

    fn hex_bytes(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn signed_crx3_verifies_and_tampering_fails() {
        let (crx, id) = signed_crx(false);
        assert_eq!(verify_crx(&crx, &id).unwrap(), b"PK\x03\x04payload");
        assert!(verify_crx(&crx, "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").is_err());
        let (bad, id) = signed_crx(true);
        assert!(verify_crx(&bad, &id).is_err());
    }

    #[test]
    fn extension_id_uses_letters_a_to_p() {
        let id = extension_id(b"key");
        assert_eq!(id.len(), 32);
        assert!(id.bytes().all(|b| (b'a'..=b'p').contains(&b)));
    }

    #[test]
    fn zip_slip_is_refused() {
        use std::io::Write;
        let mut buffer = std::io::Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut buffer);
            let options = zip::write::SimpleFileOptions::default();
            writer.start_file("../escape.txt", options).unwrap();
            writer.write_all(b"x").unwrap();
            writer.finish().unwrap();
        }
        let dest = std::env::temp_dir().join("agentgo-zip-slip-test");
        let _ = std::fs::remove_dir_all(&dest);
        assert!(extract_zip(buffer.get_ref(), &dest).is_err());
        assert!(!dest.parent().unwrap().join("escape.txt").exists());
        let _ = std::fs::remove_dir_all(&dest);
    }

    #[test]
    fn zip_extracts_normal_files() {
        use std::io::Write;
        let mut buffer = std::io::Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut buffer);
            let options = zip::write::SimpleFileOptions::default();
            writer.start_file("a/b.txt", options).unwrap();
            writer.write_all(b"hello").unwrap();
            writer.finish().unwrap();
        }
        let dest = std::env::temp_dir().join("agentgo-zip-ok-test");
        let _ = std::fs::remove_dir_all(&dest);
        assert_eq!(extract_zip(buffer.get_ref(), &dest), Ok(1));
        assert_eq!(
            std::fs::read(dest.join("a").join("b.txt")).unwrap(),
            b"hello"
        );
        let _ = std::fs::remove_dir_all(&dest);
    }
}
