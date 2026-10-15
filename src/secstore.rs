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
//! * `fvmk1:<hex>` - macOS only (r75): AES-256-GCM, key = HKDF(random master
//!   key held in the login Keychain). A stolen password file AND a stolen
//!   identity file are worthless without the user's Keychain. Old `fvk1:`
//!   values are upgraded on the first load; `FV_NO_KEYCHAIN=1` switches the
//!   Keychain off.
//! * `fvk1:<hex>` - AES-256-GCM, key = HKDF(identity secret). Used on Linux,
//!   as fallback if DPAPI/Keychain fail, and as the legacy macOS format. A
//!   stolen password file on its own is worthless, the identity file is the
//!   key.
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
const TAG_MK: &str = "fvmk1:";
/// Extra DPAPI entropy: another program using DPAPI cannot open our blobs
/// by accident.
#[cfg(windows)]
const DPAPI_ENTROPY: &[u8] = b"freeviewer-v1 host password";

/// Is this stored value already in a protected form?
pub fn is_sealed(stored: &str) -> bool {
    stored.starts_with(TAG_DPAPI) || stored.starts_with(TAG_KEY) || stored.starts_with(TAG_MK)
}

/// Should the caller rewrite this stored value? True for legacy plain text
/// and, on a Mac with a usable Keychain, for the old identity-key format
/// (`fvk1:`, migration to the Keychain, r75). Callers must only rewrite when
/// they could open ALL values of the file.
pub fn needs_reseal(stored: &str) -> bool {
    needs_reseal_with(stored, keychain_usable())
}

fn needs_reseal_with(stored: &str, keychain_ok: bool) -> bool {
    if stored.is_empty() {
        return false;
    }
    !is_sealed(stored) || (keychain_ok && stored.starts_with(TAG_KEY))
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
    if let Some(s) = seal_mk(plain) {
        return s;
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
    if let Some(h) = stored.strip_prefix(TAG_MK) {
        return open_mk(h);
    }
    if let Some(h) = stored.strip_prefix(TAG_KEY) {
        return open_key(h);
    }
    Some(stored.to_string())
}

fn derive_key() -> [u8; 32] {
    let secret = crate::ident::load_or_create_secret();
    hkdf_key(secret.as_bytes(), b"freeviewer-v1 host password")
}

fn hkdf_key(ikm: &[u8], info: &[u8]) -> [u8; 32] {
    let hk = Hkdf::<Sha256>::new(None, ikm);
    let mut key = [0u8; 32];
    hk.expand(info, &mut key).expect("hkdf expand");
    key
}

/// AES-256-GCM, output = tag + hex(nonce || ciphertext).
fn aead_seal(key: &[u8; 32], tag: &str, plain: &str) -> Option<String> {
    let aead = Aes256Gcm::new_from_slice(key).ok()?;
    let nonce = random_bytes(12);
    let ct = aead
        .encrypt(Nonce::from_slice(&nonce), plain.as_bytes())
        .ok()?;
    let mut out = nonce;
    out.extend_from_slice(&ct);
    Some(format!("{}{}", tag, hex::encode(out)))
}

/// Reverse of [`aead_seal`] for the hex part (tag already stripped).
fn aead_open(key: &[u8; 32], h: &str) -> Option<String> {
    let raw = hex::decode(h).ok()?;
    if raw.len() < 13 {
        return None;
    }
    let aead = Aes256Gcm::new_from_slice(key).ok()?;
    let pt = aead.decrypt(Nonce::from_slice(&raw[..12]), &raw[12..]).ok()?;
    String::from_utf8(pt).ok()
}

fn seal_key(plain: &str) -> Option<String> {
    aead_seal(&derive_key(), TAG_KEY, plain)
}

fn open_key(h: &str) -> Option<String> {
    aead_open(&derive_key(), h)
}

const KEYCHAIN_INFO: &[u8] = b"freeviewer-v1 host password keychain";

fn mk_key(master: &[u8]) -> [u8; 32] {
    hkdf_key(master, KEYCHAIN_INFO)
}

fn seal_mk(plain: &str) -> Option<String> {
    let master = keychain::master_key()?;
    aead_seal(&mk_key(&master), TAG_MK, plain)
}

fn open_mk(h: &str) -> Option<String> {
    let master = keychain::master_key()?;
    aead_open(&mk_key(&master), h)
}

/// Can sealed values be tied to the macOS Keychain right now?
fn keychain_usable() -> bool {
    keychain::master_key().is_some()
}

/// macOS Keychain (r75): one random 32 byte master key lives in the login
/// Keychain as a generic password; the password files only hold values
/// encrypted with a key derived from it. Never overwrites an existing item:
/// if reading fails for any reason except "not found" (locked, access
/// denied) the result is `None` and callers fall back to `fvk1:`.
#[cfg(target_os = "macos")]
mod keychain {
    use std::sync::Mutex;

    const SERVICE: &str = "com.freeviewer.secstore";
    const ACCOUNT: &str = "host-password-master-key";
    /// errSecItemNotFound
    const NOT_FOUND: i32 = -25300;

    static CACHE: Mutex<Option<[u8; 32]>> = Mutex::new(None);

    pub fn master_key() -> Option<[u8; 32]> {
        if std::env::var("FV_NO_KEYCHAIN").map(|v| v == "1").unwrap_or(false) {
            return None;
        }
        let mut cache = CACHE.lock().ok()?;
        if let Some(k) = *cache {
            return Some(k);
        }
        let key = match security_framework::passwords::get_generic_password(SERVICE, ACCOUNT) {
            Ok(v) if v.len() == 32 => {
                let mut k = [0u8; 32];
                k.copy_from_slice(&v);
                k
            }
            // a damaged item of the wrong size is NOT replaced: it may be
            // somebody else's data, and replacing would orphan sealed files
            Ok(_) => return None,
            Err(e) if e.code() == NOT_FOUND => {
                let fresh = crate::crypto::random_bytes(32);
                security_framework::passwords::set_generic_password(SERVICE, ACCOUNT, &fresh)
                    .ok()?;
                let mut k = [0u8; 32];
                k.copy_from_slice(&fresh);
                k
            }
            Err(_) => return None,
        };
        *cache = Some(key);
        Some(key)
    }
}

#[cfg(not(target_os = "macos"))]
mod keychain {
    pub fn master_key() -> Option<[u8; 32]> {
        None
    }
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
    ///
    /// # Safety
    /// `out` must come from a successful `CryptProtectData`/`CryptUnprotectData` call; its buffer is
    /// system-allocated and freed here, so it must not be used afterwards.
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
        // SAFETY: `input`/`entropy` borrow `plain` and the static entropy, which outlive the call; `out` is a local the API fills, and `take` runs only after `.ok()?` succeeded.
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
        unprotect_with(sealed, super::DPAPI_ENTROPY)
    }

    pub fn unprotect_with(sealed: &[u8], entropy: &[u8]) -> Option<Vec<u8>> {
        let input = blob(sealed);
        let entropy = blob(entropy);
        let mut out = CRYPT_INTEGER_BLOB::default();
        // SAFETY: same as in `protect`: the blobs borrow data that outlives the call, `take` runs only after a successful call.
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

    #[test]
    fn keychain_tag_roundtrip_with_a_master_key() {
        let master = [7u8; 32];
        let key = mk_key(&master);
        let s = aead_seal(&key, TAG_MK, "mac secret pw").unwrap();
        assert!(s.starts_with("fvmk1:") && is_sealed(&s));
        assert!(!s.contains("secret"));
        assert_eq!(
            aead_open(&key, s.strip_prefix(TAG_MK).unwrap()).as_deref(),
            Some("mac secret pw")
        );
        // another master key (other Keychain) cannot open it
        assert!(aead_open(&mk_key(&[8u8; 32]), s.strip_prefix(TAG_MK).unwrap()).is_none());
        // the identity key (fvk1) and the keychain key differ
        let _d = own_dir("mk-vs-id");
        assert_ne!(mk_key(&[1u8; 32]), derive_key());
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn keychain_values_do_not_open_without_a_keychain() {
        let _d = own_dir("mk-nokc");
        assert!(open("fvmk1:00112233445566778899aabbccddeeff00").is_none());
        // and sealing never produces them here
        assert!(seal("abcd1234").starts_with(if cfg!(windows) { "fvdp1:" } else { "fvk1:" }));
    }

    #[test]
    fn reseal_rules() {
        // legacy plain text always migrates, empty never
        assert!(needs_reseal_with("plainpw", false));
        assert!(needs_reseal_with("plainpw", true));
        assert!(!needs_reseal_with("", true));
        // fvk1 upgrades only when a Keychain is usable
        assert!(!needs_reseal_with("fvk1:00", false));
        assert!(needs_reseal_with("fvk1:00", true));
        // already in the best format / DPAPI: leave alone
        assert!(!needs_reseal_with("fvmk1:00", true));
        assert!(!needs_reseal_with("fvdp1:00", true));
    }

    #[test]
    fn legacy_fvk1_still_opens_after_the_keychain_change() {
        let _d = own_dir("legacy-fvk1");
        let s = seal_key("oldmacpw123").unwrap();
        assert_eq!(open(&s).as_deref(), Some("oldmacpw123"));
    }

    // ---- real Windows only (run the cross-compiled test exe on a PC, r73) ----

    #[cfg(windows)]
    #[test]
    fn win_dpapi_roundtrip_and_format() {
        let _d = own_dir("win-rt");
        let s = seal("windows pw 123");
        assert!(s.starts_with("fvdp1:"), "DPAPI not used: {}", &s[..8.min(s.len())]);
        assert!(!s.contains("windows"));
        assert_eq!(open(&s).as_deref(), Some("windows pw 123"));
        assert_ne!(s, seal("windows pw 123"));
    }

    #[cfg(windows)]
    #[test]
    fn win_dpapi_wrong_entropy_and_tampering_fail() {
        let _d = own_dir("win-bad");
        let s = seal("windows pw 123");
        let raw = hex::decode(s.strip_prefix(TAG_DPAPI).unwrap()).unwrap();
        assert!(dpapi::unprotect_with(&raw, b"other program").is_none());
        assert!(dpapi::unprotect_with(&raw, DPAPI_ENTROPY).is_some());
        let mut bad = raw.clone();
        let n = bad.len() - 1;
        bad[n] ^= 0x55;
        assert!(dpapi::unprotect(&bad).is_none());
        assert!(open("fvdp1:zz").is_none());
    }

    #[cfg(windows)]
    #[test]
    fn win_migration_plain_file_becomes_sealed() {
        let d = own_dir("win-mig");
        let f = d.join("password.txt");
        std::fs::write(&f, "plain-win-pw-1\r\n").unwrap();
        // same steps as ident::fixed_password()
        let raw = std::fs::read_to_string(&f).unwrap();
        let raw = raw.trim().to_string();
        assert!(needs_reseal(&raw));
        let plain = open(&raw).unwrap();
        assert_eq!(plain, "plain-win-pw-1");
        write_sealed_file(&f, &plain).unwrap();
        let after = std::fs::read_to_string(&f).unwrap();
        assert!(after.starts_with("fvdp1:"), "not migrated: {}", after);
        assert!(!after.contains("plain-win"));
        assert!(!needs_reseal(&after));
        assert_eq!(open(&after).as_deref(), Some("plain-win-pw-1"));
    }

    /// Cross-account half 1: run as SYSTEM (scheduled task), env FV_XACCT_FILE.
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn win_xacct_write() {
        let f = std::env::var("FV_XACCT_FILE").expect("FV_XACCT_FILE");
        write_sealed_file(std::path::Path::new(&f), "xacct-secret-42").unwrap();
        let raw = std::fs::read_to_string(&f).unwrap();
        assert!(raw.starts_with("fvdp1:"));
    }

    /// Cross-account half 2: run as the normal user, same file.
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn win_xacct_read() {
        let f = std::env::var("FV_XACCT_FILE").expect("FV_XACCT_FILE");
        let raw = std::fs::read_to_string(&f).unwrap();
        assert_eq!(open(raw.trim()).as_deref(), Some("xacct-secret-42"));
    }
}
