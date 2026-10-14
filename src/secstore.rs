//! At-rest protection for the permanent host passwords (roadmap r10 / r64).
//!
//! The host must recompute Argon2id(password, session salt) for every
//! session, so a one-way hash is impossible without changing the protocol.
//! Instead the stored form is *encrypted* with a key the host can recreate:
//!
//! * `fvdp1:<hex>` - Windows DPAPI, machine scope (the service runs as
//!   SYSTEM, the UI as the user, both must read the same file). Anybody
//!   logged in on this PC can decrypt it; a copied file/backup/disk image
//!   from another PC cannot.
//! * `fvk1:<hex>` - AES-256-GCM, key = HKDF(identity secret). Used on
//!   macOS/Linux (keychain stub) and as fallback if DPAPI fails. A stolen
//!   password file on its own is worthless, the identity file is the key.
//! * anything else = legacy plain text. It is still read (migration) and is
//!   rewritten sealed by the callers on first load.
//!
//! The wire protocol and the host-proof vectors are untouched: only the
//! storage format changes.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use hkdf::Hkdf;
use sha2::Sha256;

use crate::crypto::random_bytes;

const TAG_DPAPI: &str = "fvdp1:";
const TAG_KEY: &str = "fvk1:";
/// Extra DPAPI entropy: another program using DPAPI cannot open our blobs
/// by accident.
#[cfg(windows)]
const DPAPI_ENTROPY: &[u8] = b"freeviewer-v1 host password";

/// Is this stored value already in a protected form?
pub fn is_sealed(stored: &str) -> bool {
    stored.starts_with(TAG_DPAPI) || stored.starts_with(TAG_KEY)
}

/// Protects `plain` for storage. If no protection is possible the plain text
/// is returned unchanged (never lose a password because sealing failed).
pub fn seal(plain: &str) -> String {
    if plain.is_empty() || is_sealed(plain) {
        return plain.to_string();
    }
    #[cfg(windows)]
    if let Some(b) = dpapi::protect(plain.as_bytes()) {
        return format!("{}{}", TAG_DPAPI, hex::encode(b));
    }
    match seal_key(plain) {
        Some(s) => s,
        None => plain.to_string(),
    }
}

/// Reverses [`seal`]. Legacy plain text is returned as is. `None` = a sealed
/// value that cannot be opened here (wrong machine, damaged).
pub fn open(stored: &str) -> Option<String> {
    if let Some(h) = stored.strip_prefix(TAG_DPAPI) {
        let raw = hex::decode(h).ok()?;
        #[cfg(windows)]
        {
            let pt = dpapi::unprotect(&raw)?;
            return String::from_utf8(pt).ok();
        }
        #[cfg(not(windows))]
        {
            let _ = raw;
            return None;
        }
    }
    if let Some(h) = stored.strip_prefix(TAG_KEY) {
        return open_key(h);
    }
    Some(stored.to_string())
}

fn derive_key() -> [u8; 32] {
    let secret = crate::ident::load_or_create_secret();
    let hk = Hkdf::<Sha256>::new(None, secret.as_bytes());
    let mut key = [0u8; 32];
    hk.expand(b"freeviewer-v1 host password", &mut key)
        .expect("hkdf expand");
    key
}

fn seal_key(plain: &str) -> Option<String> {
    let aead = Aes256Gcm::new_from_slice(&derive_key()).ok()?;
    let nonce = random_bytes(12);
    let ct = aead
        .encrypt(Nonce::from_slice(&nonce), plain.as_bytes())
        .ok()?;
    let mut out = nonce;
    out.extend_from_slice(&ct);
    Some(format!("{}{}", TAG_KEY, hex::encode(out)))
}

fn open_key(h: &str) -> Option<String> {
    let raw = hex::decode(h).ok()?;
    if raw.len() < 13 {
        return None;
    }
    let aead = Aes256Gcm::new_from_slice(&derive_key()).ok()?;
    let pt = aead.decrypt(Nonce::from_slice(&raw[..12]), &raw[12..]).ok()?;
    String::from_utf8(pt).ok()
}

/// Writes a secret file: sealed content, owner-only permissions (0600) where
/// the platform has them. Windows relies on the folder ACL plus DPAPI.
pub fn write_sealed_file(path: &std::path::Path, plain: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    write_private(path, &seal(plain))
}

/// Plain write of an already sealed value with 0600 permissions.
pub fn write_private(path: &std::path::Path, content: &str) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        // write a sibling and rename: a crash never leaves a half file
        let tmp = path.with_extension("tmp");
        let _ = std::fs::remove_file(&tmp);
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(content.as_bytes())?;
        f.sync_all()?;
        drop(f);
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
        std::fs::rename(&tmp, path)
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, content)
    }
}

/// Constant-time equality for the "type the old password" check.
pub fn same_secret(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let mut diff = (a.len() ^ b.len()) as u8;
    for i in 0..a.len().max(b.len()) {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(windows)]
mod dpapi {
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPTPROTECT_LOCAL_MACHINE,
        CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    fn blob(data: &[u8]) -> CRYPT_INTEGER_BLOB {
        CRYPT_INTEGER_BLOB {
            cbData: data.len() as u32,
            pbData: data.as_ptr() as *mut u8,
        }
    }

    /// Copies the system allocated output and frees it.
    unsafe fn take(out: CRYPT_INTEGER_BLOB) -> Vec<u8> {
        let v = if out.pbData.is_null() {
            Vec::new()
        } else {
            std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec()
        };
        if !out.pbData.is_null() {
            let _ = LocalFree(HLOCAL(out.pbData as *mut core::ffi::c_void));
        }
        v
    }

    pub fn protect(plain: &[u8]) -> Option<Vec<u8>> {
        let input = blob(plain);
        let entropy = blob(super::DPAPI_ENTROPY);
        let mut out = CRYPT_INTEGER_BLOB::default();
        unsafe {
            CryptProtectData(
                &input,
                windows::core::PCWSTR::null(),
                Some(&entropy),
                None,
                None,
                CRYPTPROTECT_LOCAL_MACHINE | CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
            .ok()?;
            Some(take(out))
        }
    }

    pub fn unprotect(sealed: &[u8]) -> Option<Vec<u8>> {
        let input = blob(sealed);
        let entropy = blob(super::DPAPI_ENTROPY);
        let mut out = CRYPT_INTEGER_BLOB::default();
        unsafe {
            CryptUnprotectData(
                &input,
                None,
                Some(&entropy),
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
            .ok()?;
            Some(take(out))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn own_dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("fv-sec-{}-{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&d);
        crate::ident::set_test_config_dir(d.clone());
        d
    }

    #[test]
    fn seal_roundtrip_hides_the_plain_text() {
        let _d = own_dir("roundtrip");
        let s = seal("correct horse battery");
        assert!(is_sealed(&s));
        assert!(!s.contains("correct"));
        assert_eq!(open(&s).as_deref(), Some("correct horse battery"));
        // two seals of the same password differ (random nonce / DPAPI salt)
        assert_ne!(s, seal("correct horse battery"));
    }

    #[test]
    fn legacy_plain_text_is_still_readable() {
        let _d = own_dir("legacy");
        assert!(!is_sealed("oldplainpw"));
        assert_eq!(open("oldplainpw").as_deref(), Some("oldplainpw"));
    }

    #[test]
    fn sealed_twice_stays_sealed_once() {
        let _d = own_dir("twice");
        let s = seal("abcd1234");
        assert_eq!(seal(&s), s);
    }

    #[test]
    fn damaged_sealed_value_does_not_open() {
        let _d = own_dir("damaged");
        let mut s = seal_key("abcd1234").unwrap();
        s.pop();
        s.push('0');
        // either a flipped nibble breaks the tag, or the value was already 0
        let ok = open(&s);
        assert!(ok.is_none() || ok.as_deref() == Some("abcd1234"));
        assert!(open("fvk1:zz").is_none());
        assert!(open("fvk1:00").is_none());
    }

    #[test]
    fn other_identity_cannot_open() {
        let d = own_dir("other-a");
        let s = seal_key("abcd1234").unwrap();
        // a different machine = a different identity secret
        let _ = std::fs::remove_file(d.join("identity.txt"));
        let d2 = own_dir("other-b");
        let _ = d2;
        assert!(open(&s).is_none());
    }

    #[test]
    fn constant_time_compare() {
        assert!(same_secret("abc", "abc"));
        assert!(!same_secret("abc", "abd"));
        assert!(!same_secret("abc", "abcd"));
        assert!(!same_secret("", "a"));
    }

    #[cfg(unix)]
    #[test]
    fn secret_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let d = own_dir("perm");
        let f = d.join("password.txt");
        write_sealed_file(&f, "abcd1234").unwrap();
        let mode = std::fs::metadata(&f).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let content = std::fs::read_to_string(&f).unwrap();
        assert!(!content.contains("abcd1234"));
    }
}
